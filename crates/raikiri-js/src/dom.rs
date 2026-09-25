//! Replaceable JavaScript-to-DOM adapter.
//!
//! Scripts use the same small DOM facade regardless of whether it is backed by
//! the current layout snapshot adapter or a future live Raikiri DOM document
//! binding. Element reads and writes always cross [`DomBackend`], so layout and
//! mutation behavior does not live in test-specific JavaScript.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{
    Context, Finalize, JsData, JsError, JsNativeError, JsResult, JsValue, NativeFunction, Source,
    Trace, js_string,
};
use cssparser::{Parser, ParserInput};

/// Opaque identifier for a node owned by a [`DomBackend`].
pub type DomNodeId = u64;

/// Geometry used by the initial WPT layout-snapshot backend.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ElementGeometry {
    /// CSSOM `offsetHeight`, rounded to an integer CSS pixel by the producer.
    pub offset_height: f64,
    /// Measured `getBoundingClientRect()` border-box geometry.
    pub bounding_client_rect: DomRect,
}

/// One element and its measured geometry in the initial snapshot backend.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SnapshotNode {
    /// Parent element handle; `None` when the parent is not an exposed element.
    pub parent: Option<DomNodeId>,
    /// Measured geometry. Elements without a layout box use the CSSOM zero rect.
    pub geometry: ElementGeometry,
}

/// A compact snapshot of the parsed element tree and its measured geometry.
///
/// This is only a convenient input for the initial snapshot backend. A live
/// DOM backend should implement [`DomBackend`] directly instead of building
/// this snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DomSnapshot {
    /// In-document elements keyed by stable host node handle.
    pub nodes: BTreeMap<DomNodeId, SnapshotNode>,
    /// HTML `id` attribute to its first matching node handle.
    pub elements_by_id: BTreeMap<String, DomNodeId>,
    /// Handle of the document body element, when present.
    pub body: Option<DomNodeId>,
}

/// The geometry returned by `Element.getBoundingClientRect()`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DomRect {
    /// Distance from the viewport's left edge in CSS pixels.
    pub left: f64,
    /// Distance from the viewport's top edge in CSS pixels.
    pub top: f64,
    /// Distance from the viewport's right edge in CSS pixels.
    pub right: f64,
    /// Distance from the viewport's bottom edge in CSS pixels.
    pub bottom: f64,
    /// Rectangle width in CSS pixels.
    pub width: f64,
    /// Rectangle height in CSS pixels.
    pub height: f64,
}

/// Host-side operations exposed through the JavaScript `document` facade.
///
/// The current WPT backend implements this trait using measured layout values.
/// A live adapter can instead keep node IDs into Raikiri DOM document and
/// perform style/layout work on geometry reads. The JavaScript surface does
/// not depend on either backend.
pub trait DomBackend: 'static {
    /// Find an element with a matching HTML `id` attribute.
    fn get_element_by_id(&mut self, id: &str) -> Result<Option<DomNodeId>, String>;

    /// Find the first element matching a CSS selector.
    fn query_selector(&mut self, selector: &str) -> Result<Option<DomNodeId>, String>;

    /// Return a node's parent, if one exists.
    fn parent_node(&mut self, node: DomNodeId) -> Result<Option<DomNodeId>, String>;

    /// Read `HTMLElement.offsetHeight`. Implementations may flush layout here.
    fn offset_height(&mut self, node: DomNodeId) -> Result<f64, String>;

    /// Read `Element.getBoundingClientRect()`. Implementations may flush layout here.
    fn bounding_client_rect(&mut self, node: DomNodeId) -> Result<DomRect, String>;

    /// Read an element's `innerHTML`.
    fn inner_html(&mut self, node: DomNodeId) -> Result<String, String>;

    /// Set an element's `innerHTML` and apply or queue the resulting DOM mutation.
    fn set_inner_html(&mut self, node: DomNodeId, value: &str) -> Result<(), String>;

    /// Read an attribute value. Return `None` when the attribute is absent.
    ///
    /// The default reports that attribute access is unsupported, preserving
    /// compatibility for backends that do not expose attributes.
    fn get_attribute(&mut self, _node: DomNodeId, _name: &str) -> Result<Option<String>, String> {
        Err("attribute reads are not supported by this DOM backend".into())
    }

    /// Check whether an attribute is present, including an empty-valued one.
    ///
    /// The default uses [`DomBackend::get_attribute`], so a backend only needs
    /// to override this method when it can answer presence more directly.
    fn has_attribute(&mut self, node: DomNodeId, name: &str) -> Result<bool, String> {
        Ok(self.get_attribute(node, name)?.is_some())
    }

    /// Set an attribute's value.
    ///
    /// The default reports that attribute mutation is unsupported.
    fn set_attribute(&mut self, _node: DomNodeId, _name: &str, _value: &str) -> Result<(), String> {
        Err("attribute mutation is not supported by this DOM backend".into())
    }

    /// Remove an attribute.
    ///
    /// The default reports that attribute mutation is unsupported.
    fn remove_attribute(&mut self, _node: DomNodeId, _name: &str) -> Result<(), String> {
        Err("attribute mutation is not supported by this DOM backend".into())
    }

    /// Create a detached HTML element with the requested local name.
    fn create_element(&mut self, _local_name: &str) -> Result<DomNodeId, String> {
        Err("element creation is not supported by this DOM backend".into())
    }

    /// Append `child` to `parent`, moving it from any existing parent.
    fn append_child(&mut self, _parent: DomNodeId, _child: DomNodeId) -> Result<(), String> {
        Err("child insertion is not supported by this DOM backend".into())
    }

    /// Read the concatenated descendant text of an element.
    fn text_content(&mut self, _node: DomNodeId) -> Result<String, String> {
        Err("textContent is not supported by this DOM backend".into())
    }

    /// Replace an element's children with text content.
    fn set_text_content(&mut self, _node: DomNodeId, _value: &str) -> Result<(), String> {
        Err("textContent mutation is not supported by this DOM backend".into())
    }

    /// Read an inline style property, or return an empty string if unset.
    fn style_property(&mut self, node: DomNodeId, property: &str) -> Result<String, String>;

    /// Read a computed value for a CSS property, or `None` when it is not exposed.
    fn computed_style_property(
        &mut self,
        _node: DomNodeId,
        _property: &str,
    ) -> Result<Option<String>, String> {
        Err("computed style reads are not supported by this DOM backend".into())
    }

    /// Set an inline style property.
    fn set_style_property(
        &mut self,
        node: DomNodeId,
        property: &str,
        value: &str,
    ) -> Result<(), String>;
}

/// Why a general JavaScript evaluation failed.
#[derive(Debug)]
pub enum ScriptError {
    /// Boa reported an uncaught JavaScript error.
    JavaScript(String),
    /// A DOM operation or layout read failed in the host backend.
    Dom(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JavaScript(message) => write!(f, "JavaScript error: {message}"),
            Self::Dom(message) => write!(f, "DOM/layout error: {message}"),
        }
    }
}

impl std::error::Error for ScriptError {}

/// A Boa context with the replaceable DOM facade installed.
///
/// Evaluate inline WPT script blocks in order on one runtime to preserve their
/// shared global scope. Testharness compatibility functions are optional and
/// can be loaded separately by the caller.
pub struct JsRuntime {
    context: Context,
    backend: BackendHandle,
}

impl JsRuntime {
    /// Create a runtime backed by the supplied host DOM implementation.
    pub fn new<B: DomBackend>(backend: B) -> Result<Self, ScriptError> {
        let mut context = Context::default();
        let backend = install(&mut context, backend)
            .map_err(|error| ScriptError::JavaScript(error.to_string()))?;
        Ok(Self { context, backend })
    }

    /// Evaluate one script in this runtime's shared global scope.
    pub fn evaluate(&mut self, source: &str) -> Result<JsValue, ScriptError> {
        let result = self.context.eval(Source::from_bytes(source.as_bytes()));
        if let Some(error) = self.backend.take_error() {
            return Err(ScriptError::Dom(error));
        }
        result.map_err(|error| ScriptError::JavaScript(error.to_string()))
    }

    pub(crate) fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }
}

/// Evaluate a script once with the supplied DOM backend.
///
/// Use [`JsRuntime`] when a page has multiple script blocks that share global
/// variables or when more than one evaluation is needed.
pub fn run_script<B: DomBackend>(source: &str, backend: B) -> Result<JsValue, ScriptError> {
    JsRuntime::new(backend)?.evaluate(source)
}

const DOM_FACADE: &str = r#"
(function (host) {
var __raikiri_elements = Object.create(null);
var __raikiri_node_ids = new WeakMap();
function __raikiri_css_name(property) {
    return String(property).replace(/[A-Z]/g, function (letter) {
        return "-" + letter.toLowerCase();
    });
}
function __raikiri_class_list(node) {
    return {
        add: function () {
            var classes = (host.getAttribute(node, "class") || "").split(/\s+/);
            for (var i = 0; i < arguments.length; i++) {
                var token = String(arguments[i]);
                if (token === "" || /\s/.test(token)) {
                    throw new TypeError("classList.add expects a non-empty token without whitespace");
                }
                if (classes.indexOf(token) < 0) classes.push(token);
            }
            host.setAttribute(node, "class", classes.filter(Boolean).join(" "));
        }
    };
}
function __raikiri_style(node) {
    var methods = {
        getPropertyValue: function (property) {
            return host.styleProperty(node, String(property));
        },
        setProperty: function (property, value) {
            host.setStyleProperty(node, String(property), String(value));
        },
        removeProperty: function (property) {
            var previous = host.styleProperty(node, String(property));
            host.setStyleProperty(node, String(property), "");
            return previous;
        }
    };
    return new Proxy(methods, {
        get: function (target, property, receiver) {
            if (typeof property !== "string" || property in target) {
                return Reflect.get(target, property, receiver);
            }
            return host.styleProperty(node, __raikiri_css_name(property));
        },
        set: function (target, property, value, receiver) {
            if (typeof property !== "string" || property in target) {
                return Reflect.set(target, property, value, receiver);
            }
            host.setStyleProperty(node, __raikiri_css_name(property), String(value));
            return true;
        }
    });
}
function __raikiri_computed_style(node) {
    return new Proxy({}, {
        get: function (target, property, receiver) {
            if (property === "getPropertyValue") {
                return function (name) {
                    return host.computedStyleProperty(node, String(name)) || "";
                };
            }
            if (typeof property !== "string" || property in target) {
                return Reflect.get(target, property, receiver);
            }
            return host.computedStyleProperty(node, __raikiri_css_name(property)) || "";
        },
        has: function (target, property) {
            if (typeof property !== "string") return property in target;
            return property in target ||
                host.computedStyleProperty(node, __raikiri_css_name(property)) !== null;
        }
    });
}
function __raikiri_element(node) {
    if (node === null || node === undefined) return null;
    var key = String(node);
    if (Object.prototype.hasOwnProperty.call(__raikiri_elements, key)) {
        return __raikiri_elements[key];
    }
    var element = {};
    var style = __raikiri_style(node);
    var classList = __raikiri_class_list(node);
    Object.defineProperties(element, {
        offsetHeight: {
            enumerable: true,
            get: function () { return host.offsetHeight(node); }
        },
        innerHTML: {
            enumerable: true,
            get: function () { return host.innerHTML(node); },
            set: function (value) { host.setInnerHTML(node, String(value)); }
        },
        textContent: {
            enumerable: true,
            get: function () { return host.textContent(node); },
            set: function (value) {
                host.setTextContent(node, value === null ? "" : String(value));
            }
        },
        classList: {
            enumerable: true,
            get: function () { return classList; }
        },
        parentNode: {
            enumerable: true,
            get: function () { return __raikiri_element(host.parentNode(node)); }
        },
        style: {
            enumerable: true,
            get: function () { return style; }
        }
    });
    element.appendChild = function (child) {
        host.appendChild(node, __raikiri_node_ids.get(child));
        return child;
    };
    element.getBoundingClientRect = function () {
        return host.boundingClientRect(node);
    };
    element.getAttribute = function (name) {
        return host.getAttribute(node, String(name));
    };
    element.hasAttribute = function (name) {
        return host.hasAttribute(node, String(name));
    };
    element.setAttribute = function (name, value) {
        host.setAttribute(node, String(name), String(value));
    };
    element.removeAttribute = function (name) {
        host.removeAttribute(node, String(name));
    };
    __raikiri_node_ids.set(element, node);
    __raikiri_elements[key] = element;
    return element;
}
var document = {
    getElementById: function (id) {
        return __raikiri_element(host.getElementById(String(id)));
    },
    querySelector: function (selector) {
        return __raikiri_element(host.querySelector(String(selector)));
    },
    createElement: function (localName) {
        return __raikiri_element(host.createElement(String(localName)));
    }
};
Object.defineProperty(document, "body", {
    enumerable: true,
    get: function () { return document.querySelector("body"); }
});
Object.defineProperty(document, "head", {
    enumerable: true,
    get: function () { return document.querySelector("head"); }
});
globalThis.document = document;
globalThis.getComputedStyle = function (element) {
    var node = __raikiri_node_ids.get(element);
    if (node === undefined) throw new TypeError("getComputedStyle expects a known element");
    return __raikiri_computed_style(node);
};
globalThis.CSS = {
    supports: function (property, value) {
        return host.cssSupports(String(property), String(value));
    }
};
globalThis.window = globalThis;
})(globalThis.__raikiri_host);
delete globalThis.__raikiri_host;
"#;

#[derive(Trace, Finalize, JsData)]
struct DomHostObject {
    #[unsafe_ignore_trace]
    backend: BackendHandle,
}

impl std::fmt::Debug for DomHostObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DomHostObject").finish_non_exhaustive()
    }
}

struct BackendState {
    backend: Box<dyn DomBackend>,
    first_error: Option<String>,
}

/// Shared state for one installed DOM backend.
#[derive(Clone)]
pub(crate) struct BackendHandle(Rc<RefCell<BackendState>>);

impl BackendHandle {
    /// Take the first backend error recorded while JavaScript was running.
    pub(crate) fn take_error(&self) -> Option<String> {
        self.0.borrow_mut().first_error.take()
    }
}

/// Install a replaceable DOM backend and the JavaScript `document` facade.
pub(crate) fn install<B: DomBackend>(context: &mut Context, backend: B) -> JsResult<BackendHandle> {
    let shared = BackendHandle(Rc::new(RefCell::new(BackendState {
        backend: Box::new(backend),
        first_error: None,
    })));
    let mut host = ObjectInitializer::with_native_data(
        DomHostObject {
            backend: shared.clone(),
        },
        context,
    );
    host.function(
        NativeFunction::from_fn_ptr(host_get_element_by_id),
        js_string!("getElementById"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(host_query_selector),
        js_string!("querySelector"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(host_parent_node),
        js_string!("parentNode"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(host_offset_height),
        js_string!("offsetHeight"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(host_bounding_client_rect),
        js_string!("boundingClientRect"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(host_inner_html),
        js_string!("innerHTML"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(host_set_inner_html),
        js_string!("setInnerHTML"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(host_create_element),
        js_string!("createElement"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(host_append_child),
        js_string!("appendChild"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(host_text_content),
        js_string!("textContent"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(host_set_text_content),
        js_string!("setTextContent"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(host_get_attribute),
        js_string!("getAttribute"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(host_has_attribute),
        js_string!("hasAttribute"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(host_set_attribute),
        js_string!("setAttribute"),
        3,
    )
    .function(
        NativeFunction::from_fn_ptr(host_remove_attribute),
        js_string!("removeAttribute"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(host_style_property),
        js_string!("styleProperty"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(host_set_style_property),
        js_string!("setStyleProperty"),
        3,
    )
    .function(
        NativeFunction::from_fn_ptr(host_computed_style_property),
        js_string!("computedStyleProperty"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(host_css_supports),
        js_string!("cssSupports"),
        2,
    );
    let host_object = host.build();
    context
        .global_object()
        .set(js_string!("__raikiri_host"), host_object, true, context)?;
    context.eval(Source::from_bytes(DOM_FACADE))?;
    Ok(shared)
}

fn shared_backend(this: &JsValue) -> JsResult<BackendHandle> {
    let Some(object) = this.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("DOM host method called with an invalid receiver")
            .into());
    };
    let Some(data) = object.downcast_ref::<DomHostObject>() else {
        return Err(JsNativeError::typ()
            .with_message("DOM host method called with an invalid receiver")
            .into());
    };
    Ok(data.backend.clone())
}

fn backend_call<T>(
    this: &JsValue,
    operation: impl FnOnce(&mut dyn DomBackend) -> Result<T, String>,
) -> JsResult<T> {
    let backend = shared_backend(this)?;
    let mut state = backend.0.borrow_mut();
    match operation(state.backend.as_mut()) {
        Ok(value) => Ok(value),
        Err(error) => {
            if state.first_error.is_none() {
                state.first_error = Some(error.clone());
            }
            Err(host_error(error))
        }
    }
}

fn node_id(argument: Option<&JsValue>, context: &mut Context) -> JsResult<DomNodeId> {
    let value = string_argument(argument, context)?;
    value.parse().map_err(|_| {
        JsNativeError::typ()
            .with_message("invalid DOM node handle")
            .into()
    })
}

fn string_argument(argument: Option<&JsValue>, context: &mut Context) -> JsResult<String> {
    Ok(argument
        .cloned()
        .unwrap_or_default()
        .to_string(context)?
        .to_std_string_escaped())
}

fn host_error(error: String) -> JsError {
    JsNativeError::error().with_message(error).into()
}

fn optional_node_id(node: Option<DomNodeId>) -> JsValue {
    node.map_or_else(JsValue::null, |id| {
        let id = id.to_string();
        JsValue::from(js_string!(id))
    })
}

fn host_get_element_by_id(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id = string_argument(args.first(), context)?;
    let result = backend_call(this, |backend| backend.get_element_by_id(&id))?;
    Ok(optional_node_id(result))
}

fn host_query_selector(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = string_argument(args.first(), context)?;
    let result = backend_call(this, |backend| backend.query_selector(&selector))?;
    Ok(optional_node_id(result))
}

fn host_parent_node(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let result = backend_call(this, |backend| backend.parent_node(node))?;
    Ok(optional_node_id(result))
}

fn host_offset_height(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let height = backend_call(this, |backend| backend.offset_height(node))?;
    Ok(JsValue::new(height))
}

fn host_bounding_client_rect(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let rect = backend_call(this, |backend| backend.bounding_client_rect(node))?;
    let mut value = ObjectInitializer::new(context);
    value
        .property(js_string!("x"), rect.left, Attribute::all())
        .property(js_string!("left"), rect.left, Attribute::all())
        .property(js_string!("y"), rect.top, Attribute::all())
        .property(js_string!("top"), rect.top, Attribute::all())
        .property(js_string!("right"), rect.right, Attribute::all())
        .property(js_string!("bottom"), rect.bottom, Attribute::all())
        .property(js_string!("width"), rect.width, Attribute::all())
        .property(js_string!("height"), rect.height, Attribute::all());
    Ok(value.build().into())
}

fn host_inner_html(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let html = backend_call(this, |backend| backend.inner_html(node))?;
    Ok(JsValue::from(js_string!(html)))
}

fn host_set_inner_html(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let html = string_argument(args.get(1), context)?;
    backend_call(this, |backend| backend.set_inner_html(node, &html))?;
    Ok(JsValue::undefined())
}

fn host_create_element(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let local_name = string_argument(args.first(), context)?;
    let node = backend_call(this, |backend| backend.create_element(&local_name))?;
    Ok(optional_node_id(Some(node)))
}

fn host_append_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = node_id(args.first(), context)?;
    let child = node_id(args.get(1), context)?;
    backend_call(this, |backend| backend.append_child(parent, child))?;
    Ok(JsValue::undefined())
}

fn host_text_content(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let text = backend_call(this, |backend| backend.text_content(node))?;
    Ok(JsValue::from(js_string!(text)))
}

fn host_set_text_content(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let text = string_argument(args.get(1), context)?;
    backend_call(this, |backend| backend.set_text_content(node, &text))?;
    Ok(JsValue::undefined())
}

fn host_get_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let name = string_argument(args.get(1), context)?;
    let value = backend_call(this, |backend| backend.get_attribute(node, &name))?;
    Ok(value.map_or_else(JsValue::null, |value| JsValue::from(js_string!(value))))
}

fn host_has_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let name = string_argument(args.get(1), context)?;
    let present = backend_call(this, |backend| backend.has_attribute(node, &name))?;
    Ok(JsValue::new(present))
}

fn host_set_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let name = string_argument(args.get(1), context)?;
    let value = string_argument(args.get(2), context)?;
    backend_call(this, |backend| backend.set_attribute(node, &name, &value))?;
    Ok(JsValue::undefined())
}

fn host_remove_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let name = string_argument(args.get(1), context)?;
    backend_call(this, |backend| backend.remove_attribute(node, &name))?;
    Ok(JsValue::undefined())
}

fn host_computed_style_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let property = string_argument(args.get(1), context)?;
    let value = backend_call(this, |backend| {
        backend.computed_style_property(node, &property)
    })?;
    Ok(value.map_or_else(JsValue::null, |value| JsValue::from(js_string!(value))))
}

fn host_css_supports(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let property = string_argument(args.first(), context)?;
    let value = string_argument(args.get(1), context)?;
    let mut input = ParserInput::new(&value);
    let mut parser = Parser::new(&mut input);
    let parsed_value_supported =
        parser
            .parse_entirely(
                |input| -> Result<
                    raikiri_style::property::PropertyValue,
                    cssparser::ParseError<'_, ()>,
                > {
                    raikiri_style::property::parse_value(&property, input)
                        .ok_or_else(|| input.new_custom_error(()))
                },
            )
            .is_ok();
    let css_wide_keyword = ["inherit", "initial", "unset", "revert", "revert-layer"]
        .iter()
        .any(|keyword| value.trim().eq_ignore_ascii_case(keyword));
    let supported = parsed_value_supported
        || (css_wide_keyword && raikiri_style::property::is_supported_property_name(&property));
    Ok(JsValue::new(supported))
}

fn host_style_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let property = string_argument(args.get(1), context)?;
    let value = backend_call(this, |backend| backend.style_property(node, &property))?;
    Ok(JsValue::from(js_string!(value)))
}

fn host_set_style_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = node_id(args.first(), context)?;
    let property = string_argument(args.get(1), context)?;
    let value = string_argument(args.get(2), context)?;
    backend_call(this, |backend| {
        backend.set_style_property(node, &property, &value)
    })?;
    Ok(JsValue::undefined())
}

#[cfg(test)]
mod tests;
