use crate::runtime::style::{attribute_names, inline_style_value, with_inline_style_property};
use crate::runtime::test_host::StubHost;
use crate::runtime::{DomRuntime, RuntimeError};

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

#[test]
fn inline_style_helpers_match_the_previous_runner_semantics() {
    assert_eq!(
        inline_style_value(Some("color: red; color: blue !important"), "color", false),
        "blue"
    );
    assert_eq!(inline_style_value(Some("--x: 1"), "--X", false), "");
    assert_eq!(inline_style_value(None, "color", false), "");
    assert_eq!(
        with_inline_style_property(Some("color: red; margin: 0"), "COLOR", "blue", false),
        "margin: 0; COLOR: blue;"
    );
    assert_eq!(
        with_inline_style_property(Some("color: red;"), "color", " ", false),
        ""
    );
    assert_eq!(
        with_inline_style_property(Some("color: red;"), "  ", "blue", false),
        "color: red;"
    );
}

#[test]
fn stylesheet_rule_declaration_updates_the_live_style_element() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = 'span { color: cyan; } span.active { color: red; }'; \
         document.head.appendChild(styleElement); \
         var sheets = document.styleSheets; var sheet = sheets[0]; \
         var rules = sheet.cssRules; var rule = rules[0]; var declaration = rule.style;",
    )
    .unwrap();
    ok(
        &mut rt,
        "sheets === document.styleSheets && sheets.length === 1 && \
         sheets.item(0) === sheet && rules.length === 2 && \
         rules.item(0) === rule && rule.selectorText === 'span' && \
         declaration.color === 'cyan' && declaration.parentRule === rule",
    );

    rt.evaluate("declaration.backgroundColor = 'magenta';")
        .unwrap();
    ok(&mut rt, "declaration.backgroundColor === 'magenta'");
    ok(
        &mut rt,
        "styleElement.textContent.includes('background-color: magenta;')",
    );
    ok(&mut rt, "rule.style === declaration");

    rt.evaluate(
        "var secondStyle = document.createElement('style'); \
         secondStyle.textContent = 'p { color: blue; }'; \
         document.head.appendChild(secondStyle);",
    )
    .unwrap();
    ok(
        &mut rt,
        "sheets.length === 2 && sheets[1].cssRules[0].selectorText === 'p'",
    );
}

#[test]
fn attribute_names_covers_plain_dashed_camel_and_webkit_forms() {
    assert_eq!(attribute_names("color"), vec!["color".to_owned()]);
    assert_eq!(
        attribute_names("font-size"),
        vec!["font-size".to_owned(), "fontSize".to_owned()]
    );
    assert_eq!(
        attribute_names("-webkit-transform"),
        vec![
            "-webkit-transform".to_owned(),
            "WebkitTransform".to_owned(),
            "webkitTransform".to_owned(),
        ]
    );
}

#[test]
fn style_object_reads_and_writes_camel_and_dashed_names() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.body.style; s.textAlign = 'center'; s.setProperty('word-spacing', '2px');")
        .unwrap();
    ok(&mut rt, "s === document.body.style");
    ok(
        &mut rt,
        "s.getPropertyValue('text-align') === 'center' && s.wordSpacing === '2px'",
    );
    ok(
        &mut rt,
        "s.removeProperty('text-align') === 'center' && s.textAlign === ''",
    );
    ok(
        &mut rt,
        "'getPropertyValue' in s && !('made-up-property' in s)",
    );
    ok(
        &mut rt,
        "var before = document.body.getAttribute('style'); s.setProperty('', 'x'); document.body.getAttribute('style') === before",
    );
}

/// Dashed and camelCase keys are real accessors on
/// `CSSStyleDeclaration.prototype` (not a named-property `Proxy`), so
/// `s instanceof CSSStyleDeclaration` and `in` work through ordinary
/// prototype-chain lookup, and an unsupported name becomes a plain expando
/// rather than a style write.
#[test]
fn interface_shape_matches_cssom() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.body.style;").unwrap();
    ok(
        &mut rt,
        "s instanceof CSSStyleDeclaration && \
         ('marginTop' in CSSStyleDeclaration.prototype) && \
         ('margin-top' in CSSStyleDeclaration.prototype) && \
         s.parentRule === null",
    );
    ok(&mut rt, "String(s) === '[object CSSStyleDeclaration]'");
    ok(
        &mut rt,
        "Object.prototype.toString.call(s) === '[object CSSStyleDeclaration]'",
    );
    ok(
        &mut rt,
        "s.fooBar = 'x'; s.getPropertyValue('foo-bar') === '' && s.fooBar === 'x'",
    );
}

#[test]
fn css_float_aliases_the_float_property() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var s = document.body.style; \
         s.cssFloat = 'left'; \
         s.getPropertyValue('float') === 'left' && s.cssFloat === 'left'",
    );
}

#[test]
fn css_text_getter_and_setter_replace_the_whole_declaration_block() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.body.style; s.marginTop = '1px';")
        .unwrap();
    ok(&mut rt, "s.cssText === 'margin-top: 1px;'");
    ok(
        &mut rt,
        "s.cssText = 'padding: 3px'; \
         s.paddingTop !== undefined && \
         s.getPropertyValue('padding') === '3px' && \
         s.marginTop === ''",
    );
    // The same literal text set directly through the attribute reads back
    // identically through `getPropertyValue` -- `cssText`'s setter stores
    // it verbatim, the same as `setAttribute`.
    ok(
        &mut rt,
        "document.body.setAttribute('style', 'padding: 3px'); \
         document.body.style.getPropertyValue('padding') === s.getPropertyValue('padding')",
    );
}

#[test]
fn element_style_assignment_forwards_to_css_text_in_strict_mode() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "'use strict'; \
         var body = document.body; \
         var declaration = body.style; \
         body.style = 'color: red; margin-top: 2px';",
    )
    .unwrap();
    ok(
        &mut rt,
        "body.style === declaration \
         && declaration.cssText === 'color: red; margin-top: 2px;' \
         && body.getAttribute('style') === declaration.cssText",
    );
}

#[test]
fn length_and_item_enumerate_declared_properties_in_order() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.body.style; s.marginTop = '1px'; s['margin-left'] = '2px';")
        .unwrap();
    ok(
        &mut rt,
        "s.getPropertyValue('margin-top') === '1px' && \
         s.marginLeft === '2px' && \
         s.length === 2 && \
         s.item(0) === 'margin-top' && \
         s.item(1) === 'margin-left' && \
         s.item(5) === '' && \
         s[0] === 'margin-top' && \
         s[5] === undefined",
    );
}

#[test]
fn get_property_priority_reports_important() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var s = document.body.style; \
         s.setProperty('color', 'red', 'important'); \
         s.getPropertyPriority('color') === 'important' && \
         s.getPropertyValue('color') === 'red'",
    );
    ok(&mut rt, "s.getPropertyPriority('no-such-prop') === ''");
}

#[test]
fn set_property_priority_argument_variants() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    // Missing, `undefined`, and `null` priority all behave like `""`.
    ok(
        &mut rt,
        "var s = document.body.style; \
         s.setProperty('color', 'red'); \
         s.getPropertyPriority('color') === ''",
    );
    ok(
        &mut rt,
        "s.setProperty('color', 'green', undefined); \
         s.getPropertyPriority('color') === ''",
    );
    ok(
        &mut rt,
        "s.setProperty('color', 'blue', null); \
         s.getPropertyPriority('color') === '' && s.getPropertyValue('color') === 'blue'",
    );
    // A bogus priority is spec-silent: no write happens at all.
    ok(
        &mut rt,
        "s.setProperty('color', 'purple', 'bogus'); \
         s.getPropertyValue('color') === 'blue'",
    );
    // An empty value removes the declaration outright, even with a
    // (would-be) `!important` priority -- it must never store a lone
    // `\" !important\"`.
    ok(
        &mut rt,
        "s.setProperty('color', '', 'important'); s.getPropertyValue('color') === ''",
    );
}

/// CSSOM marks `setProperty`'s `value` `[LegacyNullToEmptyString]`: an
/// explicit `null` converts to `""`, which then removes the declaration
/// the same as an ordinary empty string would.
#[test]
fn set_property_value_null_removes_the_declaration() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var s = document.body.style; \
         s.setProperty('color', 'red'); \
         s.setProperty('color', null); \
         s.getPropertyValue('color') === ''",
    );
}

/// `cssFloat` is `[LegacyNullToEmptyString] CSSOMString` (CSSOM
/// `CSSStyleProperties`), the same as `setProperty`'s `value`.
#[test]
fn css_float_setter_treats_null_as_empty_string() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var s = document.body.style; \
         s.cssFloat = 'left'; \
         s.cssFloat = null; \
         s.getPropertyValue('float') === ''",
    );
}

/// Every generated camel-cased/dashed/webkit-cased attribute setter is
/// also `[LegacyNullToEmptyString] CSSOMString` (CSSOM §6.7.1's three
/// partial-interface blocks) -- checked through both a camelCase key and
/// its independently-installed dashed twin.
#[test]
fn generated_attribute_setters_treat_null_as_empty_string() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var s = document.body.style; \
         s.marginTop = '1px'; \
         s.marginTop = null; \
         s.getPropertyValue('margin-top') === ''",
    );
    ok(
        &mut rt,
        "s['margin-left'] = '2px'; \
         s['margin-left'] = null; \
         s.getPropertyValue('margin-left') === ''",
    );
}

/// WebIDL converts every argument before an operation's own algorithm
/// steps run: a `priority` whose `toString` throws must abort
/// `setProperty` even when `value` is empty, and the declaration must be
/// left exactly as it was -- the empty-`value` removal never runs, because
/// the throwing conversion happens first.
#[test]
fn set_property_converts_priority_before_the_empty_value_check() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.body.style; s.setProperty('color', 'red');")
        .unwrap();
    let error = rt
        .evaluate(
            "s.setProperty('color', '', { toString: function () { throw new Error('boom'); } });",
        )
        .unwrap_err();
    match error {
        RuntimeError::JavaScript(message) => assert!(message.contains("boom"), "{message}"),
        other => panic!("expected a JavaScript error, got {other:?}"),
    }
    ok(&mut rt, "s.getPropertyValue('color') === 'red'");
}

#[test]
fn style_objects_behave_like_ordinary_objects_for_inherited_members() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "String(document.body.style) === '[object CSSStyleDeclaration]'",
    );
    ok(
        &mut rt,
        "'' + document.body.style === '[object CSSStyleDeclaration]'",
    );
    // `getPropertyValue` now lives on `CSSStyleDeclaration.prototype`
    // rather than being an own property of every instance (the earlier
    // per-instance Proxy defined it directly on each target); the
    // inherited member is still callable without throwing.
    ok(
        &mut rt,
        "!getComputedStyle(document.body).hasOwnProperty('getPropertyValue') && \
         typeof getComputedStyle(document.body).getPropertyValue === 'function'",
    );
}

#[test]
fn geometry_reads_flush_only_when_dirty() {
    let (mut host, _, _, body) = StubHost::page();
    host.geometry.insert(body, StubHost::rect(20.4));
    let flushes = host.flushes.clone();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.offsetHeight === 20 && document.body.offsetWidth === 10",
    );
    ok(
        &mut rt,
        "var r = document.body.getBoundingClientRect(); \
         r.x === 1 && r.y === 2 && r.left === 1 && r.top === 2 && \
         r.right === 11 && r.bottom === 22.4 && r.width === 10 && r.height === 20.4",
    );
    assert_eq!(
        flushes.get(),
        1,
        "initial dirty state flushes once, repeated reads reuse it"
    );
    rt.evaluate("document.body.setAttribute('class', 'z'); document.body.offsetHeight;")
        .unwrap();
    assert_eq!(flushes.get(), 2);
}

#[test]
fn boxless_elements_report_the_zero_rect() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var r = document.head.getBoundingClientRect(); r.width === 0 && r.height === 0 && document.head.offsetHeight === 0",
    );
}

#[test]
fn computed_style_unsupported_property_skips_flush() {
    let (mut host, _, _, body) = StubHost::page();
    host.computed
        .insert((body, "white-space".into()), "normal".into());
    let flushes = host.flushes.clone();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "getComputedStyle(document.body).getPropertyValue('no-such-prop') === ''",
    );
    assert_eq!(flushes.get(), 0);
    ok(
        &mut rt,
        "var cs = getComputedStyle(document.body); cs.whiteSpace === 'normal' && cs.getPropertyValue('white-space') === 'normal' && ('whiteSpace' in cs)",
    );
    assert_eq!(flushes.get(), 1);
    // `noSuchProp` is not a supported CSS property name at all, so unlike
    // `whiteSpace` above it has no accessor on the prototype either.
    ok(&mut rt, "!('noSuchProp' in cs)");
}

/// A dashed CSS property name works the same as its camelCase accessor,
/// since both are real, independently-defined keys on
/// `CSSStyleDeclaration.prototype`.
#[test]
fn computed_style_supports_dashed_property_name_access() {
    let (mut host, _, _, body) = StubHost::page();
    host.computed
        .insert((body, "white-space".into()), "pre".into());
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var cs = getComputedStyle(document.body); \
         ('white-space' in cs) && cs['white-space'] === 'pre'",
    );
}

#[test]
fn get_computed_style_rejects_a_non_element_argument() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let error = rt.evaluate("getComputedStyle(document)").unwrap_err();
    match error {
        RuntimeError::JavaScript(message) => {
            assert!(message.contains("argument is not an Element"), "{message}");
        }
        other => panic!("expected a JavaScript TypeError, got {other:?}"),
    }
}

#[test]
fn get_computed_style_returns_the_same_object_per_element() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "getComputedStyle(document.body) === getComputedStyle(document.body)",
    );
    ok(
        &mut rt,
        "getComputedStyle(document.body) !== getComputedStyle(document.head)",
    );
}

#[test]
fn computed_declaration_indexed_view_lists_longhands_only_lexicographic() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let expected = crate::runtime::style::computed_longhand_names().len();
    ok(
        &mut rt,
        &format!(
            "var cs = getComputedStyle(document.body); \
             cs.length === {expected} && cs.item(0) === 'align-content' && cs[0] === 'align-content'",
        ),
    );
    // Shorthands are excluded (CSSOM §6.7: longhands only).
    ok(
        &mut rt,
        "var found = false; \
         for (var i = 0; i < cs.length; i++) { \
           var n = cs.item(i); \
           if (n === 'margin' || n === 'padding' || n === 'background' || n === 'border' || \
               n === 'font' || n === 'flex' || n === 'gap' || n === 'grid' || \
               n === 'overflow' || n === 'outline' || n === 'columns' || \
               n === 'border-top' || n === 'border-right' || n === 'border-bottom' || \
               n === 'border-left') { found = true; } \
         } \
         !found",
    );
    // Lexicographic order (already sorted in the backing list).
    ok(
        &mut rt,
        "var sorted = true; \
         for (var i = 1; i < cs.length; i++) { if (cs.item(i - 1) > cs.item(i)) { sorted = false; } } \
         sorted",
    );
}

#[test]
fn computed_declaration_write_paths_throw_no_modification_allowed() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var cs = getComputedStyle(document.body);")
        .unwrap();
    for (label, script) in [
        ("setter", "cs.color = 'red'"),
        ("setProperty", "cs.setProperty('color', 'red')"),
        ("removeProperty", "cs.removeProperty('color')"),
        ("cssText setter", "cs.cssText = 'color: red'"),
        ("cssFloat setter", "cs.cssFloat = 'left'"),
    ] {
        let src = format!(
            "try {{ {script}; false }} catch (e) {{ e.name === 'NoModificationAllowedError' }}"
        );
        ok(&mut rt, &src);
        // The write attempt must not have taken effect either.
        assert!(
            rt.evaluate("document.body.getAttribute('style')")
                .unwrap()
                .is_null(),
            "{label} must not have written the style attribute"
        );
    }
}

#[test]
fn computed_css_text_is_always_empty_and_priority_is_never_important() {
    let (mut host, _, _, body) = StubHost::page();
    host.computed.insert((body, "color".into()), "red".into());
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var cs = getComputedStyle(document.body); \
         cs.cssText === '' && \
         cs.getPropertyPriority('color') === '' && \
         cs.cssFloat === ''",
    );
}

#[test]
fn computed_value_failure_is_a_host_error() {
    let (mut host, _, _, body) = StubHost::page();
    host.computed
        .insert((body, "white-space".into()), "normal".into());
    host.fail_computed = true;
    let mut rt = DomRuntime::new(host).unwrap();
    assert_eq!(
        rt.evaluate("getComputedStyle(document.body).getPropertyValue('white-space')"),
        Err(RuntimeError::Host("stub computed style failure".into()))
    );
}

/// A real host's `computed_value` failure message can embed a raikiri-dom
/// arena index (the same concern `Element.innerHTML`'s own host-failure path
/// documents, see `node.rs`'s `inner_html`); that index is an implementation
/// detail and must never be observable from script. Modeled here with a host
/// whose failure message names the node id directly, since the stub host's
/// own `fail_computed` message carries no index.
#[test]
fn get_computed_style_js_visible_message_hides_the_node_index() {
    use std::any::Any;

    use super::super::host::{BoxGeometry, DocumentHost, HostError};

    struct FailingComputed(StubHost);
    impl DocumentHost for FailingComputed {
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
            _property: &str,
        ) -> Result<Option<String>, HostError> {
            Err(HostError(format!("no computed value for node {node}")))
        }
        fn parse_fragment(
            &mut self,
            tag: &str,
            ns: &str,
            markup: &str,
        ) -> Result<raikiri_dom::Document, HostError> {
            self.0.parse_fragment(tag, ns, markup)
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

    let (host, _, _, body) = StubHost::page();
    let mut rt = DomRuntime::new(FailingComputed(host)).unwrap();
    // The overall `evaluate` call still reports a host failure (the recorded
    // detail, which really does name the node), so the JS-visible message is
    // stashed into a global for a second, unrelated `evaluate` call to read
    // back.
    let _ = rt.evaluate(
        "var caughtMessage = ''; \
         try { getComputedStyle(document.body).getPropertyValue('white-space'); } \
         catch (e) { caughtMessage = e.message; }",
    );
    let message = rt
        .evaluate("caughtMessage")
        .unwrap()
        .to_string(rt.context_mut())
        .unwrap()
        .to_std_string_escaped();
    assert!(
        !message.chars().any(|c| c.is_ascii_digit()),
        "node index leaked into the JS-visible message: {message:?}"
    );
    let err = rt.evaluate("getComputedStyle(document.body).getPropertyValue('white-space')");
    assert_eq!(
        err,
        Err(RuntimeError::Host(format!(
            "no computed value for node {body}"
        )))
    );
}

#[test]
fn flush_failure_is_a_host_error() {
    let (mut host, ..) = StubHost::page();
    host.fail_flush = true;
    let mut rt = DomRuntime::new(host).unwrap();
    assert_eq!(
        rt.evaluate("document.body.offsetHeight"),
        Err(RuntimeError::Host("stub flush failure".into()))
    );
}

#[test]
fn geometry_failure_is_a_host_error() {
    let (mut host, ..) = StubHost::page();
    host.fail_geometry = true;
    let mut rt = DomRuntime::new(host).unwrap();
    assert_eq!(
        rt.evaluate("document.body.offsetHeight"),
        Err(RuntimeError::Host("stub geometry failure".into()))
    );
}

#[test]
fn style_expandos_do_not_touch_the_style_attribute() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.style[Symbol.iterator] === undefined",
    );
    ok(
        &mut rt,
        "(document.body.style[Symbol()] = 'x', document.body.getAttribute('style') === null)",
    );
    ok(
        &mut rt,
        "var cs = getComputedStyle(document.body); !(Symbol() in cs)",
    );
}

#[test]
fn css_supports_uses_the_value_parser() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "CSS.supports('color', 'red') && !CSS.supports('color', '12px') && CSS.supports('color', 'inherit')",
    );
    ok(&mut rt, "!CSS.supports('no-such-property', 'inherit')");
    // Shorthands that expand into other properties during parsing are
    // still supported names for the CSS-wide-keyword branch.
    ok(&mut rt, "CSS.supports('border-radius', 'inherit') === true");
    ok(&mut rt, "CSS.supports('grid-area', 'initial') === true");
}

/// CSSOM `setProperty` (and every attribute setter, which calls it): a
/// value that does not parse for a supported property is dropped without
/// touching the existing declaration.
#[test]
fn setters_ignore_values_that_do_not_parse_for_the_property() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var e = document.createElement('div'); var s = e.style;")
        .unwrap();
    ok(
        &mut rt,
        "s['box-sizing'] = 'border-box'; s['box-sizing'] = 'margin-box'; \
         s.boxSizing = 'bogus'; s.setProperty('box-sizing', 'nope', 'important'); \
         s.cssFloat = 'sideways'; \
         s.getPropertyValue('box-sizing') === 'border-box' \
         && e.getAttribute('style') === 'box-sizing: border-box;'",
    );
    ok(&mut rt, "s.color = 'red !important'; s.color === ''");
}

/// A value that does parse is stored in its canonical serialization;
/// CSS-wide keywords serialize ASCII-lowercased and custom properties are
/// stored trimmed.
///
/// CSSOM `setProperty` step 2.2: a non-custom name outside the supported
/// list makes the method return without a write.
#[test]
fn setters_store_the_canonical_serialization_of_a_parsed_value() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.createElement('div').style;")
        .unwrap();
    ok(&mut rt, "s.paddingTop = '010.0px'; s.paddingTop === '10px'");
    ok(
        &mut rt,
        "s.setProperty('COLOR', '#234'); s.color === 'rgb(34, 51, 68)'",
    );
    ok(&mut rt, "s.color = 'lab(0 0 0)'; s.color === 'lab(0 0 0)'");
    ok(&mut rt, "s.color = 'INHERIT'; s.color === 'inherit'");
    ok(
        &mut rt,
        "s.setProperty('--x', ' anything { } '); s.getPropertyValue('--x') === 'anything { }'",
    );
    ok(
        &mut rt,
        "s.setProperty('not-a-property', 'whatever'); \
         s.getPropertyValue('not-a-property') === '' && s.length === 3",
    );
}

/// CSSOM `setProperty` step 2.1: a non-custom `property` name is used ASCII
/// -lowercased, so the name that ends up stored (and later shows up in
/// `item()`/`cssText`) is lowercase even when the caller passed a
/// differently-cased name. This is purely a *name* normalization: the value
/// itself was already parsed and serialized case-insensitively before this
/// fix (`raikiri_style::property::parse_value` resolves the property by its
/// own lowercased copy of the name), so the stored value is unaffected.
#[test]
fn set_property_lowercases_a_non_custom_property_name_before_storing_it() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.createElement('div').style; s.setProperty('COLOR', '#234');")
        .unwrap();
    ok(
        &mut rt,
        "s.item(0) === 'color' && s.cssText === 'color: rgb(34, 51, 68);'",
    );
}

/// The same step explicitly excludes custom property names (`--`-prefixed):
/// `setProperty`/`removeProperty`/`getPropertyPriority` must use those
/// exactly as given, never lowercased.
#[test]
fn set_property_does_not_lowercase_a_custom_property_name() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.createElement('div').style; s.setProperty('--Foo', '1px');")
        .unwrap();
    ok(
        &mut rt,
        "s.item(0) === '--Foo' && \
         s.getPropertyValue('--Foo') === '1px' && \
         s.getPropertyValue('--foo') === ''",
    );
}

/// `removeProperty` already removed a declaration regardless of the input
/// name's case (`same_property`'s existing ASCII case-insensitive compare),
/// so the removal itself is not new behavior here -- what step 2.1 changes
/// is that the name `setProperty` stored, and so what `item()` reports right
/// up until the removal, is lowercase rather than whatever case the caller
/// used.
#[test]
fn remove_property_finds_a_differently_cased_stored_name() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.createElement('div').style; s.setProperty('MARGIN-TOP', '1px');")
        .unwrap();
    ok(&mut rt, "s.item(0) === 'margin-top'");
    ok(
        &mut rt,
        "s.removeProperty('margin-TOP') === '1px' && s.cssText === ''",
    );
}

/// Shorthands that expand entirely into other properties (and so have no
/// value type of their own) are still validated against their grammar.
#[test]
fn setters_validate_expanding_shorthands_too() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.createElement('div').style;")
        .unwrap();
    ok(
        &mut rt,
        "s.borderRadius = '-1px'; s.gridGap = 'auto'; s.setProperty('grid', 'none none'); \
         s.borderRadius === '' && s.gridGap === '' && s.grid === ''",
    );
    ok(&mut rt, "s.borderRadius = '1px'; s.borderRadius !== ''");
}

/// CSSOM §6.7 step 2.2: `setProperty` with an unsupported non-custom name
/// returns without a write -- no storage, no length change, and the
/// priority/value reads stay empty.
#[test]
fn set_property_rejects_an_unsupported_non_custom_name() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var e = document.createElement('div'); var s = e.style; s.setProperty('color', 'red');",
    )
    .unwrap();
    ok(
        &mut rt,
        "s.setProperty('not-a-property', 'whatever'); \
         s.getPropertyValue('not-a-property') === '' && \
         s.getPropertyPriority('not-a-property') === '' && \
         s.removeProperty('not-a-property') === '' && \
         s.length === 1 && s.item(0) === 'color'",
    );
    // A mixed-case unsupported name is lowercased first, then still rejected.
    ok(
        &mut rt,
        "s.setProperty('NOT-A-PROPERTY', 'whatever'); s.length === 1",
    );
}

/// CSSOM keeps `setProperty`'s `value` and `priority` separate: a `value`
/// with a trailing `!important` is invalid and leaves the declaration
/// untouched, rather than becoming a priority.
#[test]
fn set_property_value_with_important_is_invalid() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.createElement('div').style; s.setProperty('color', 'red');")
        .unwrap();
    ok(
        &mut rt,
        "s.setProperty('color', 'red !important'); \
         s.getPropertyValue('color') === 'red' && s.getPropertyPriority('color') === ''",
    );
    ok(
        &mut rt,
        "s.setProperty('color', 'blue !IMPORTANT'); \
         s.getPropertyValue('color') === 'red'",
    );
    // Custom values with a trailing `!important` are likewise invalid here.
    ok(
        &mut rt,
        "s.setProperty('--x', '1px'); \
         s.setProperty('--x', 'foo !important'); \
         s.getPropertyValue('--x') === '1px' && s.getPropertyPriority('--x') === ''",
    );
}

/// Inline `length`/`item` read through the validated view: unsupported
/// names and unparseable values are dropped, duplicates collapse to the
/// last occurrence (survivor at the last position), declaration order is
/// otherwise preserved.
#[test]
fn inline_length_and_item_dedup_and_drop_invalid() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var e = document.createElement('div'); var s = e.style;")
        .unwrap();
    ok(
        &mut rt,
        "e.setAttribute('style', 'color: red; color: blue'); \
         s.length === 1 && s.item(0) === 'color' && s.getPropertyValue('color') === 'blue'",
    );
    ok(
        &mut rt,
        "e.setAttribute('style', 'color: red; margin-top: 1px; color: blue'); \
         s.length === 2 && s.item(0) === 'margin-top' && s.item(1) === 'color' && \
         s.getPropertyValue('color') === 'blue'",
    );
    ok(
        &mut rt,
        "e.setAttribute('style', 'color: bogus; margin-top: 1px; foo: bar'); \
         s.length === 1 && s.item(0) === 'margin-top' && \
         s.getPropertyValue('color') === '' && s.getPropertyValue('foo') === ''",
    );
}

/// `cssText` on setting parses as a declaration block, dropping invalid
/// declarations; on getting it serializes the validated view (lowercased
/// names, canonical values, `!important` preserved).
#[test]
fn css_text_setter_parses_and_drops_invalid() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var e = document.createElement('div'); var s = e.style;")
        .unwrap();
    ok(
        &mut rt,
        "s.cssText = 'color: bogus; margin-top: 1px; foo: bar'; \
         s.length === 1 && s.item(0) === 'margin-top' && \
         s.cssText === 'margin-top: 1px;'",
    );
    ok(
        &mut rt,
        "s.cssText = 'COLOR: #234'; s.cssText === 'color: rgb(34, 51, 68);'",
    );
    ok(
        &mut rt,
        "s.cssText = 'color: red !important'; \
         s.getPropertyValue('color') === 'red' && \
         s.getPropertyPriority('color') === 'important' && \
         s.cssText === 'color: red !important;'",
    );
}

/// CSS-wide keywords serialize ASCII-lowercased.
#[test]
fn css_wide_keywords_serialize_lowercase() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.createElement('div').style;")
        .unwrap();
    ok(
        &mut rt,
        "s.setProperty('color', 'INHERIT'); s.color === 'inherit'",
    );
    ok(
        &mut rt,
        "s.setProperty('margin-top', 'Revert-Layer'); s.marginTop === 'revert-layer'",
    );
    ok(
        &mut rt,
        "s.cssText = 'color: INITIAL'; s.color === 'initial'",
    );
}

/// A whitespace-only value counts as empty and removes the declaration --
/// for custom properties (CSSOM) and, consistently, for non-custom ones.
#[test]
fn whitespace_only_value_removes_the_declaration() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.createElement('div').style;")
        .unwrap();
    ok(
        &mut rt,
        "s.setProperty('--x', '1px'); s.setProperty('--x', '   '); \
         s.getPropertyValue('--x') === '' && s.length === 0",
    );
    ok(
        &mut rt,
        "s.setProperty('color', 'red'); s.setProperty('color', '   '); \
         s.getPropertyValue('color') === '' && s.length === 0",
    );
    // Whitespace-only also removes even with a (would-be) `!important` priority.
    ok(
        &mut rt,
        "s.setProperty('--y', '1px'); s.setProperty('--y', '   ', 'important'); \
         s.getPropertyValue('--y') === ''",
    );
}

#[test]
fn get_bounding_client_rect_subtracts_viewport_except_for_fixed() {
    use crate::runtime::PositionKind;
    use crate::runtime::test_host::StubHost;
    let (mut host, html, _, body) = StubHost::page();
    let d = StubHost::dom_rect;
    // Body at (1, 2) static; a fixed child at (5, 6).
    host.geometry.insert(
        body,
        StubHost::boxed(
            d(1.0, 2.0, 10.0, 10.0),
            d(1.0, 2.0, 10.0, 10.0),
            PositionKind::Static,
        ),
    );
    let fixed = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, fixed).unwrap();
    host.geometry.insert(
        fixed,
        StubHost::boxed(
            d(5.0, 6.0, 7.0, 8.0),
            d(5.0, 6.0, 7.0, 8.0),
            PositionKind::Fixed,
        ),
    );
    // Root needs a box so document-element scroll linkage works; its own
    // rect is not asserted here.
    host.geometry.insert(
        html,
        StubHost::boxed(
            d(0.0, 0.0, 100.0, 100.0),
            d(0.0, 0.0, 100.0, 100.0),
            PositionKind::Static,
        ),
    );
    let mut rt = crate::runtime::DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.getBoundingClientRect().left === 1 && document.body.getBoundingClientRect().top === 2",
    );
    rt.evaluate("window.scrollBy(1, 2);").unwrap();
    ok(
        &mut rt,
        "document.body.getBoundingClientRect().left === 0 && document.body.getBoundingClientRect().top === 0 \
         && document.body.getBoundingClientRect().width === 10",
    );
    ok(
        &mut rt,
        "var kids = document.body.children; kids.length === 1 \
         && kids[0].getBoundingClientRect().left === 5 && kids[0].getBoundingClientRect().top === 6",
    );
}

#[test]
fn stylesheet_declaration_reads_priorities_indexes_and_writes_css_text() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@import url(\"theme.css\"); span { color: cyan !important; background-color: magenta; }'; \
         document.head.appendChild(styleElement); \
         var rules = document.styleSheets[0].cssRules; \
         var importRule = rules[0]; var rule = rules[1]; var declaration = rule.style;",
    )
    .unwrap();

    ok(
        &mut rt,
        "importRule.style === undefined && importRule.cssText === '@import url(\"theme.css\");' && \
         importRule.selectorText === '@import url(\"theme.css\")' && \
         declaration.length === 2 && declaration.item(0) === 'color' && \
         declaration.item(1) === 'background-color' && declaration.item(9) === '' && \
         rules.item(9) === null && \
         declaration.getPropertyPriority('color') === 'important' && \
         declaration.cssText === 'color: cyan !important; background-color: magenta;'",
    );

    rt.evaluate("declaration.cssText = 'color: red; margin-top: 2px';")
        .unwrap();
    ok(
        &mut rt,
        "declaration.cssText === 'color: red; margin-top: 2px;' && \
         styleElement.textContent.includes('color: red; margin-top: 2px;')",
    );
}

#[test]
fn stylesheet_objects_report_invalid_receivers_and_stale_rules() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = 'span { color: cyan; }'; \
         document.head.appendChild(styleElement); \
         var sheet = document.styleSheets[0]; var rule = sheet.cssRules[0]; \
         var declaration = rule.style;",
    )
    .unwrap();
    ok(
        &mut rt,
        "(function() { try { Object.getOwnPropertyDescriptor(sheet, 'cssRules').get.call({}); return false; } catch (_) { return true; } })() && \
         (function() { try { Object.getOwnPropertyDescriptor(rule, 'selectorText').get.call({}); return false; } catch (_) { return true; } })()",
    );

    rt.evaluate("styleElement.textContent = ''; rule.selectorText;")
        .unwrap_err();
    rt.evaluate("declaration.color = 'red';").unwrap_err();

    rt.evaluate(
        "styleElement.textContent = 'span { color: cyan; }'; \
         var replacementRule = sheet.cssRules[0]; var replacementStyle = replacementRule.style; \
         styleElement.textContent = '@import url(\"theme.css\");';",
    )
    .unwrap();
    rt.evaluate("replacementStyle.cssText = 'color: red';")
        .unwrap_err();
}

#[test]
fn stylesheet_list_finds_nested_html_and_svg_style_elements() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var nested = document.createElement('section'); \
         nested.appendChild(document.createTextNode('ordinary text')); \
         var htmlStyle = document.createElement('style'); htmlStyle.textContent = 'p { color: red; }'; \
         nested.appendChild(htmlStyle); document.body.appendChild(nested); \
         var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); \
         var svgStyle = document.createElementNS('http://www.w3.org/2000/svg', 'style'); \
         svgStyle.textContent = 'circle { fill: blue; }'; svg.appendChild(svgStyle); document.body.appendChild(svg);",
    )
    .unwrap();
    ok(
        &mut rt,
        "document.styleSheets.length === 2 && document.styleSheets.item(9) === null && \
         document.styleSheets[0].cssRules[0].selectorText === 'p' && \
         document.styleSheets[1].cssRules[0].selectorText === 'circle'",
    );
}

#[test]
fn important_is_detected_with_tokens_not_text() {
    // `! important` is still a priority, with whitespace between the tokens.
    assert_eq!(
        inline_style_value(Some("color: red ! important"), "color", false),
        "red"
    );
    // A trailing comment after `!important` does not hide it.
    assert_eq!(
        inline_style_value(Some("color: red !important /* c */"), "color", false),
        "red"
    );
    // A `!` that does not start a final `!important` invalidates the
    // declaration, as in the cascade.
    assert_eq!(
        inline_style_value(Some("color: red ! foo"), "color", false),
        ""
    );
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.setAttribute('style', 'color: red ! important; margin-top: 1px !important /* c */'); \
         var s = document.body.style; \
         s.color === 'red' && s.getPropertyPriority('color') === 'important' && \
         s.marginTop === '1px' && s.getPropertyPriority('margin-top') === 'important'",
    );
    // The priority survives a write to another property.
    ok(
        &mut rt,
        "s.paddingTop = '2px'; \
         s.getPropertyPriority('color') === 'important' && s.paddingTop === '2px'",
    );
    // A value with a spaced `! important` is still not a priority.
    ok(
        &mut rt,
        "s.setProperty('padding-top', '3px ! important'); s.paddingTop === '2px'",
    );
}

#[test]
fn css_wide_keywords_ignore_comments() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var s = document.body.style; s.setProperty('color', 'INHERIT /**/'); \
         s.color === 'inherit' && CSS.supports('color', '/**/ unset')",
    );
}

#[test]
fn serialized_declarations_keep_their_boundaries() {
    // An unclosed string in a custom property must not swallow the
    // declaration written after it.
    assert_eq!(
        with_inline_style_property(Some("--x: 'abc"), "color", "red", false),
        "--x: 'abc'; color: red;"
    );
    // A decoded name is escaped back.
    assert_eq!(
        with_inline_style_property(Some(r"--a\;b: 1"), "color", "red", false),
        r"--a\;b: 1; color: red;"
    );
    assert_eq!(
        with_inline_style_property(Some("--x: f(a [b"), "color", "red", true),
        "--x: f(a [b]); color: red !important;"
    );
    assert_eq!(
        with_inline_style_property(None, "--x", "a /* b", false),
        "--x: a /* b*/;"
    );
    assert_eq!(
        with_inline_style_property(Some("--x: url(a"), "color", "red", false),
        "--x: url(a); color: red;"
    );
    assert_eq!(
        with_inline_style_property(Some("--x: url(a\\"), "color", "red", false),
        "--x: url(a\\\u{FFFD}); color: red;"
    );
    assert_eq!(
        with_inline_style_property(Some("--x: url(a\""), "color", "red", false),
        "--x: url(a\"); color: red;"
    );
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.setAttribute('style', '--x: url(a\"'); \
         document.body.style.color = 'red'; document.body.style.color === 'red'",
    );
    assert_eq!(
        with_inline_style_property(None, "--x", "a /* b */", false),
        "--x: a /* b */;"
    );
    assert_eq!(
        with_inline_style_property(Some("--x: 'a\\"), "color", "red", false),
        "--x: 'a\\\n'; color: red;"
    );
    assert_eq!(
        with_inline_style_property(Some("--x: a\\"), "color", "red", false),
        "--x: a\\\u{FFFD}; color: red;"
    );
    // Closed values are left alone.
    assert_eq!(
        with_inline_style_property(Some("--x: f(a) 'b' url(c)"), "color", "red", false),
        "--x: f(a) 'b' url(c); color: red;"
    );

    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var s = document.body.style; s.setProperty('--x', \"'abc\"); s.setProperty('color', 'red'); \
         s.color === 'red' && s.length === 2",
    );
    // A top-level `;` in a value would split it into two declarations.
    ok(
        &mut rt,
        "s.setProperty('--y', 'a; color: blue'); s.color === 'red' && s.length === 2",
    );
}

#[test]
fn deeply_nested_values_do_not_overflow_the_stack() {
    // A value nested past the limit is dropped rather than closed.
    let deep = format!("--x: {}", "f(".repeat(20000));
    assert_eq!(
        with_inline_style_property(Some(&deep), "color", "red", false),
        "color: red;"
    );
    // Nesting within the limit is still closed.
    let shallow = format!("--x: {}", "f(".repeat(100));
    assert_eq!(
        with_inline_style_property(Some(&shallow), "color", "red", false),
        format!("{}{}; color: red;", shallow, ")".repeat(100))
    );
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var s = document.body.style; \
         document.body.setAttribute('style', '--x: ' + 'f('.repeat(20000)); \
         s.color = 'red'; var text = s.cssText; \
         s.cssText = '--y: ' + '['.repeat(20000); \
         text === 'color: red;' && s.cssText === ''",
    );
}

#[test]
fn page_rules_expose_style_and_margin_rule_insertion() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@page :first { margin: 1px; @top-left { content: \"a\" } } p { color: red; }'; \
         document.head.appendChild(styleElement); \
         var sheet = styleElement.sheet; var pageRule = sheet.rules[0];",
    )
    .unwrap();
    ok(
        &mut rt,
        "sheet === document.styleSheets[0] && sheet.rules === sheet.cssRules && \
         Object.prototype.toString.call(pageRule) === '[object CSSPageRule]' && \
         Object.prototype.toString.call(sheet.rules[1]) === '[object CSSStyleRule]' && \
         sheet.rules[1].insertRule === undefined && \
         pageRule.selectorText === ':first' && pageRule.style.margin === '1px' && \
         document.createElement('style').sheet === null && \
         Object.prototype.toString.call(styleElement) === '[object HTMLStyleElement]'",
    );

    rt.evaluate(
        "pageRule.style.setProperty('margin', '50px 0 0'); \
         pageRule.insertRule('@bottom-center { content: \"b\" }', 1); \
         pageRule.insertRule(' @top-center { content: \"c\" } ');",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.style.margin === '50px 0px 0px' && \
         styleElement.textContent.includes('@top-left { content: \"a\" }') && \
         styleElement.textContent.indexOf('@top-center') < styleElement.textContent.indexOf('@top-left') && \
         styleElement.textContent.indexOf('@top-left') < styleElement.textContent.indexOf('@bottom-center')",
    );

    rt.evaluate("pageRule.deleteRule(1);").unwrap();
    ok(
        &mut rt,
        "!styleElement.textContent.includes('@top-left') && \
         styleElement.textContent.includes('@bottom-center') && \
         sheet.rules[1].selectorText === 'p'",
    );
    ok(
        &mut rt,
        "(function() { try { pageRule.insertRule('p { color: red }'); return false; } \
           catch (e) { return e.name === 'SyntaxError'; } })() && \
         (function() { try { pageRule.insertRule('@top-left {}', 9); return false; } \
           catch (e) { return e.name === 'IndexSizeError'; } })() && \
         (function() { try { pageRule.deleteRule(9); return false; } \
           catch (e) { return e.name === 'IndexSizeError'; } })()",
    );
}

#[test]
fn page_rule_style_reads_and_writes_page_descriptors() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@page { size: A4; page-orientation: rotate-left; margin: 1px } p { size: A4 }'; \
         document.head.appendChild(styleElement); \
         var pageRule = styleElement.sheet.cssRules[0]; var styleRule = styleElement.sheet.cssRules[1];",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.style.getPropertyValue('page-orientation') === 'rotate-left' && \
         pageRule.style.getPropertyValue('size') === 'A4' && \
         pageRule.style.length === 3 && \
         pageRule.style.cssText.includes('page-orientation: rotate-left') && \
         styleRule.style.getPropertyValue('size') === '' && styleRule.style.length === 0",
    );
    rt.evaluate(
        "pageRule.style.setProperty('page-orientation', 'rotate-right'); \
         pageRule.style.setProperty('marks', 'not-a-mark'); \
         styleRule.style.setProperty('page-orientation', 'rotate-right');",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.style.getPropertyValue('page-orientation') === 'rotate-right' && \
         pageRule.style.getPropertyValue('marks') === '' && \
         styleRule.style.getPropertyValue('page-orientation') === '' && \
         !styleElement.textContent.includes('p { page-orientation')",
    );
    rt.evaluate("pageRule.style.cssText = 'bleed: 6pt; page-orientation: sideways';")
        .unwrap();
    ok(
        &mut rt,
        "pageRule.style.cssText === 'bleed: 6pt;' && pageRule.style.getPropertyPriority('bleed') === ''",
    );
}

#[test]
fn page_rule_selector_text_comes_from_tokens() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@page/**/:first {} @PAGE  named:left ,  :right {} @p\\\\61ge {}'; \
         document.head.appendChild(styleElement); var rules = styleElement.sheet.cssRules;",
    )
    .unwrap();
    ok(
        &mut rt,
        "rules[0].selectorText === ':first' && rules[1].selectorText === 'named:left , :right' && \
         Object.prototype.toString.call(rules[2]) === '[object CSSPageRule]' && rules[2].selectorText === ''",
    );
}

#[test]
fn rule_objects_follow_a_rule_kind_change_after_text_replacement() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); styleElement.textContent = 'p { color: red }'; \
         document.head.appendChild(styleElement); var before = styleElement.sheet.cssRules[0]; \
         var beforeTag = Object.prototype.toString.call(before); \
         styleElement.textContent = '@page { margin: 1px }'; var after = styleElement.sheet.cssRules[0];",
    )
    .unwrap();
    ok(
        &mut rt,
        "beforeTag === '[object CSSStyleRule]' && \
         Object.prototype.toString.call(after) === '[object CSSPageRule]' && \
         typeof after.insertRule === 'function' && after.style.margin === '1px' && \
         styleElement.sheet.cssRules[0] === after",
    );
}

#[test]
fn page_rule_css_rules_lists_margin_rules_live() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@page { margin: 1px; @TOP-LEFT { content: \"a\" } }'; \
         document.head.appendChild(styleElement); var pageRule = styleElement.sheet.cssRules[0]; \
         var list = pageRule.cssRules; var first = list[0];",
    )
    .unwrap();
    ok(
        &mut rt,
        "list === pageRule.cssRules && list.length === 1 && \
         Object.prototype.toString.call(list) === '[object CSSRuleList]' && \
         Object.prototype.toString.call(first) === '[object CSSMarginRule]' && \
         first.name === 'top-left' && first.cssText === '@TOP-LEFT { content: \"a\" }' && \
         list.item(1) === null && list[1] === undefined",
    );
    rt.evaluate(
        "pageRule.insertRule('@bottom-center { content: \"b\" }', 1); pageRule.deleteRule(0);",
    )
    .unwrap();
    ok(
        &mut rt,
        "list.length === 1 && list[0].name === 'bottom-center' && first.name === 'bottom-center'",
    );
    rt.evaluate("styleElement.textContent = 'p {}';").unwrap();
    ok(
        &mut rt,
        "list.length === 0 && first.cssText === '' && first.name === ''",
    );
}

#[test]
fn page_rule_margin_rules_follow_the_page_grammar() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@page { @foo {} @top-left x {} @top-right; @top-center { content: \"a\" } }'; \
         document.head.appendChild(styleElement); var pageRule = styleElement.sheet.cssRules[0];",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.cssRules.length === 1 && pageRule.cssRules[0].name === 'top-center'",
    );
    rt.evaluate(
        "pageRule.insertRule('@top-l\\\\65 ft {}'); pageRule.insertRule('@bottom-left/**/{}', 2);",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.cssRules.length === 3 && pageRule.cssRules[0].name === 'top-left' && \
         pageRule.cssRules[2].name === 'bottom-left' && \
         (function() { try { pageRule.insertRule('@top-left x {}'); return false; } \
           catch (e) { return e.name === 'SyntaxError'; } })()",
    );
    rt.evaluate(
        "pageRule.style.setProperty('size', '50vw'); pageRule.style.setProperty('margin', '1vh');",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.style.getPropertyValue('size') === '' && pageRule.style.getPropertyValue('margin') === ''",
    );
}

#[test]
fn svg_style_elements_expose_their_sheet() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var svgStyle = document.createElementNS('http://www.w3.org/2000/svg', 'style'); \
         svgStyle.textContent = 'rect { fill: red }'; document.body.appendChild(svgStyle);",
    )
    .unwrap();
    ok(
        &mut rt,
        "Object.prototype.toString.call(svgStyle) === '[object SVGStyleElement]' && \
         svgStyle.sheet === document.styleSheets[0] && svgStyle.sheet.cssRules[0].selectorText === 'rect'",
    );
}

#[test]
fn style_sheets_skip_non_css_style_types_and_inserts_ignore_comments() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var plain = document.createElement('style'); plain.setAttribute('type', 'text/plain'); \
         plain.textContent = 'p { color: red }'; document.head.appendChild(plain); \
         var css = document.createElement('style'); css.setAttribute('type', 'TEXT/CSS'); \
         css.textContent = '@page { margin: 1px }'; document.head.appendChild(css);",
    )
    .unwrap();
    ok(
        &mut rt,
        "plain.sheet === null && document.styleSheets.length === 1 && \
         document.styleSheets[0] === css.sheet",
    );
    rt.evaluate("plain.removeAttribute('type');").unwrap();
    ok(
        &mut rt,
        "plain.sheet !== null && document.styleSheets.length === 2",
    );
    rt.evaluate(
        "var pageRule = css.sheet.cssRules[0]; \
         pageRule.insertRule(' /* lead */ @top-left { content: \"a\" } /* tail */ ');",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.cssRules.length === 1 && pageRule.cssRules[0].cssText === '@top-left { content: \"a\" }' && \
         pageRule.style.margin === '1px' && \
         !css.textContent.includes('lead') && \
         (function() { try { pageRule.insertRule('@top-left {} x'); return false; } \
           catch (e) { return e.name === 'SyntaxError'; } })()",
    );
}

#[test]
fn page_rules_need_a_valid_page_selector() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var s = document.createElement('style'); \
         s.textContent = '@page :bogus { margin: 1px } @page named:first { margin: 2px } \
         @page f/**/oo { margin: 3px }'; \
         document.head.appendChild(s);",
    )
    .unwrap();
    ok(
        &mut rt,
        "Object.prototype.toString.call(s.sheet.cssRules[0]) !== '[object CSSPageRule]' && \
         s.sheet.cssRules[0].style === undefined && \
         Object.prototype.toString.call(s.sheet.cssRules[1]) === '[object CSSPageRule]' && \
         s.sheet.cssRules[1].selectorText === 'named:first' && \
         Object.prototype.toString.call(s.sheet.cssRules[2]) !== '[object CSSPageRule]'",
    );
}

#[test]
fn style_sheet_getter_rejects_non_style_receivers() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var st = document.createElement('style'); document.head.appendChild(st); \
         var getter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(st), 'sheet').get; \
         var thrown = null; try { getter.call(document.body); } catch (e) { thrown = e; }",
    )
    .unwrap();
    ok(
        &mut rt,
        "thrown instanceof TypeError && getter.call(st) === st.sheet",
    );
}

#[test]
fn page_rule_insert_rule_closes_blocks_left_open_at_eof() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var s = document.createElement('style'); \
         s.textContent = '@page { margin: 1px } p { color: blue }'; \
         document.head.appendChild(s); \
         s.sheet.cssRules[0].insertRule('@top-left { content: \"a');",
    )
    .unwrap();
    ok(
        &mut rt,
        "s.sheet.cssRules.length === 2 && \
         s.sheet.cssRules[0].cssRules.length === 1 && \
         s.sheet.cssRules[0].cssRules[0].name === 'top-left' && \
         s.sheet.cssRules[1].selectorText === 'p'",
    );
}

#[test]
fn folded_math_functions_keep_a_calc_wrapper() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var d = document.createElement('div'); \
         d.style.width = 'calc(10px + 20px)'; d.style.marginTop = 'MIN(1px, 2px)'; \
         d.style.height = '10px'; d.style.maxWidth = 'calc(10px + 5%)';",
    )
    .unwrap();
    ok(
        &mut rt,
        "d.style.width === 'calc(30px)' && d.style.marginTop === 'calc(1px)' && \
         d.style.height === '10px' && d.style.maxWidth === 'calc(5% + 10px)'",
    );
}

#[test]
fn calc_sums_and_content_read_back_in_canonical_form() {
    assert_eq!(
        inline_style_value(
            Some("border-spacing: calc(10px + 0.5em) calc(10px - 0.5em)"),
            "border-spacing",
            false
        ),
        "calc(0.5em + 10px) calc(-0.5em + 10px)"
    );
    assert_eq!(
        inline_style_value(Some("--x: calc(10px + 0.5em)"), "--x", false),
        "calc(10px + 0.5em)"
    );
    assert_eq!(
        inline_style_value(
            Some("content: counter(n, DECIMAL) / \"alt\" counter(n)"),
            "content",
            false
        ),
        "counter(n) / \"alt\" counter(n)"
    );
    assert_eq!(
        inline_style_value(Some("content: \"a\" / open-quote"), "content", false),
        ""
    );
}

#[test]
fn page_rule_selector_text_setter_replaces_only_a_valid_prelude() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@page :first { margin: 1px; @top-left { content: \"a\" } } p { color: red }'; \
         document.head.appendChild(styleElement); var pageRule = styleElement.sheet.cssRules[0]; \
         pageRule.selectorText = 'named:LEFT';",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.selectorText === 'named:LEFT' && \
         styleElement.textContent === '@page named:LEFT { margin: 1px; @top-left { content: \"a\" } } p { color: red }' && \
         pageRule.style.margin === '1px' && pageRule.cssRules.length === 1",
    );
    rt.evaluate("pageRule.selectorText = 'p { color: blue'; pageRule.selectorText = '::first';")
        .unwrap();
    ok(&mut rt, "pageRule.selectorText === 'named:LEFT'");
    rt.evaluate("pageRule.selectorText = '';").unwrap();
    ok(
        &mut rt,
        "pageRule.selectorText === '' && styleElement.sheet.cssRules.length === 2 && \
         styleElement.textContent.startsWith('@page { margin: 1px;')",
    );
    // A style rule's selectorText stays read-only here.
    rt.evaluate("var styleRule = styleElement.sheet.cssRules[1]; styleRule.selectorText = 'div';")
        .unwrap();
    ok(&mut rt, "styleRule.selectorText === 'p'");
}

#[test]
fn page_rule_style_exposes_descriptor_attributes() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@page { size: a4; page-orientation: rotate-left } p { color: red }'; \
         document.head.appendChild(styleElement); var style = styleElement.sheet.cssRules[0].style;",
    )
    .unwrap();
    ok(
        &mut rt,
        "style.size === 'a4' && style.pageOrientation === 'rotate-left' && \
         style['page-orientation'] === 'rotate-left' && style.marks === '' && style.bleed === ''",
    );
    rt.evaluate(
        "style.size = 'letter landscape'; style.pageOrientation = 'rotate-right'; \
         style.marks = 'crop'; style.bleed = '6pt'; style.size = 'nonsense';",
    )
    .unwrap();
    ok(
        &mut rt,
        "style.getPropertyValue('size') === 'letter landscape' && \
         style.getPropertyValue('page-orientation') === 'rotate-right' && \
         style.marks === 'crop' && style.bleed === '6pt' && \
         !Object.getOwnPropertyNames(Object.getPrototypeOf(style)).includes('pageOrientation') && \
         !('size' in styleElement.sheet.cssRules[1].style)",
    );
}

#[test]
fn margin_rule_style_reads_and_writes_its_own_block() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var styleElement = document.createElement('style'); \
         styleElement.textContent = '@page { margin: 1px; @top-left { content: \"a\"; color: red } @bottom-center { color: blue } }'; \
         document.head.appendChild(styleElement); var pageRule = styleElement.sheet.cssRules[0]; \
         var topLeft = pageRule.cssRules[0]; var style = topLeft.style;",
    )
    .unwrap();
    ok(
        &mut rt,
        "pageRule.cssRules[0] === topLeft && topLeft.style === style && \
         style.parentRule === topLeft && style.length === 2 && \
         style.content === '\"a\"' && style.color === 'red' && style.margin === '' && \
         pageRule.cssRules[1].style.color === 'blue'",
    );
    rt.evaluate("style.content = '\"b\"'; style.removeProperty('color');")
        .unwrap();
    ok(
        &mut rt,
        "style.content === '\"b\"' && style.color === '' && topLeft.cssText.indexOf('\"b\"') > 0 && \
         pageRule.style.margin === '1px' && pageRule.cssRules[1].style.color === 'blue' && \
         pageRule.cssRules.length === 2",
    );
    rt.evaluate("style.cssText = 'color: green';").unwrap();
    ok(
        &mut rt,
        "style.cssText === 'color: green;' && style.content === '' && \
         topLeft.name === 'top-left' && pageRule.cssRules[1].name === 'bottom-center'",
    );
}
