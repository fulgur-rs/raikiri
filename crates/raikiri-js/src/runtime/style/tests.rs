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
        inline_style_value(Some("color: red; color: blue !important"), "color"),
        "blue"
    );
    assert_eq!(inline_style_value(Some("--x: 1"), "--X"), "");
    assert_eq!(inline_style_value(None, "color"), "");
    assert_eq!(
        with_inline_style_property(Some("color: red; margin: 0"), "COLOR", "blue"),
        "margin: 0; COLOR: blue;"
    );
    assert_eq!(
        with_inline_style_property(Some("color: red;"), "color", " "),
        ""
    );
    assert_eq!(
        with_inline_style_property(Some("color: red;"), "  ", "blue"),
        "color: red;"
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
fn computed_declaration_indexed_view_lists_every_supported_property_name() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let expected = raikiri_style::property::supported_property_names().len();
    ok(
        &mut rt,
        &format!(
            "var cs = getComputedStyle(document.body); \
             cs.length === {expected} && cs.item(0) === 'align-content' && cs[0] === 'align-content'",
        ),
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

/// A value that does parse is stored in its canonical serialization, and
/// CSS-wide keywords and custom properties are stored as given (trimmed).
///
/// The last assertion pins a documented deviation from CSSOM `setProperty`
/// step 2 (2.2: a property that is not a supported CSS property makes the
/// method return without a write): this runtime's `setProperty` has always
/// stored such a name's value verbatim, and still does.
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
    ok(&mut rt, "s.color = 'inherit'; s.color === 'inherit'");
    ok(
        &mut rt,
        "s.setProperty('--x', ' anything { } '); s.getPropertyValue('--x') === 'anything { }'",
    );
    ok(
        &mut rt,
        "s.setProperty('not-a-property', 'whatever'); \
         s.getPropertyValue('not-a-property') === 'whatever'",
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
