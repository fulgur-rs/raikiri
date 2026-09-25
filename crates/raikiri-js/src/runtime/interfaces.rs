//! Interface objects (constructor + prototype pairs) and node wrappers.

use boa_engine::native_function::NativeFunctionPointer;
use boa_engine::object::builtins::JsFunction;
use boa_engine::object::{ConstructorBuilder, FunctionObjectBuilder, JsObject};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Trace,
    js_string,
};
use raikiri_dom::NodeKind;

use super::webidl::with_state;

const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

/// Native data of every node wrapper. The index is never exposed to scripts.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct NodeHandle {
    #[unsafe_ignore_trace]
    pub index: usize,
}

/// Native data of `DOMException` instances.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct DomExceptionData {
    #[unsafe_ignore_trace]
    pub name: String,
    #[unsafe_ignore_trace]
    pub message: String,
}

/// Prototype objects, looked up when wrapping nodes.
#[allow(
    dead_code,
    reason = "every interface prototype is kept for bindings that create or test instances"
)]
pub(crate) struct Protos {
    pub event_target: JsObject,
    pub node: JsObject,
    pub element: JsObject,
    pub html_element: JsObject,
    pub character_data: JsObject,
    pub text: JsObject,
    pub comment: JsObject,
    pub document: JsObject,
    pub document_fragment: JsObject,
    pub dom_exception: JsObject,
}

/// The prototypes registered by [`install`].
pub(crate) fn protos(context: &Context) -> &Protos {
    context
        .get_data::<Protos>()
        .expect("interfaces are installed")
}

fn illegal_constructor(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Err(JsNativeError::typ()
        .with_message("Illegal constructor")
        .into())
}

/// A native getter/setter/method function object.
pub(crate) fn function(context: &mut Context, name: &str, f: NativeFunctionPointer) -> JsFunction {
    FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(f))
        .name(JsString::from(name))
        .build()
}

/// Members of one interface prototype.
pub(crate) struct Members {
    /// Read-only attributes: `(name, getter)`.
    pub getters: &'static [(&'static str, NativeFunctionPointer)],
    /// Read-write attributes: `(name, getter, setter)`.
    pub accessors: &'static [(&'static str, NativeFunctionPointer, NativeFunctionPointer)],
    /// Operations: `(name, length, function)`.
    pub methods: &'static [(&'static str, usize, NativeFunctionPointer)],
}

/// An interface with no members of its own.
pub(crate) const NO_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[],
    methods: &[],
};

/// An interface's prototype object and interface object.
struct Interface {
    prototype: JsObject,
    constructor: JsObject,
}

/// Build interface `name`, define its members, and expose the interface
/// object on the global object.
///
/// `parent_prototype` becomes the `[[Prototype]]` of the new prototype object
/// and `parent_constructor` the `[[Prototype]]` of the interface object
/// (WebIDL §3.7.1, §3.7.3); either defaults to the ordinary intrinsic.
fn interface(
    context: &mut Context,
    name: &str,
    parent_prototype: Option<&JsObject>,
    parent_constructor: Option<&JsObject>,
    members: &Members,
) -> JsResult<Interface> {
    let getters: Vec<_> = members
        .getters
        .iter()
        .map(|&(n, f)| (n, function(context, &format!("get {n}"), f)))
        .collect();
    let accessors: Vec<_> = members
        .accessors
        .iter()
        .map(|&(n, g, s)| {
            (
                n,
                function(context, &format!("get {n}"), g),
                function(context, &format!("set {n}"), s),
            )
        })
        .collect();
    let mut builder =
        ConstructorBuilder::new(context, NativeFunction::from_fn_ptr(illegal_constructor));
    builder.name(name).length(0).constructor(true);
    if let Some(proto) = parent_prototype {
        builder.inherit(proto.clone());
    }
    if let Some(ctor) = parent_constructor {
        builder.custom_prototype(ctor.clone());
    }
    let attr = Attribute::CONFIGURABLE | Attribute::ENUMERABLE;
    for (n, getter) in getters {
        builder.accessor(JsString::from(n), Some(getter), None, attr);
    }
    for (n, getter, setter) in accessors {
        builder.accessor(JsString::from(n), Some(getter), Some(setter), attr);
    }
    for &(n, length, f) in members.methods {
        builder.method(NativeFunction::from_fn_ptr(f), JsString::from(n), length);
    }
    let standard = builder.build();
    let constructor = standard.constructor();
    let exposed = Attribute::WRITABLE | Attribute::CONFIGURABLE;
    context.register_global_property(JsString::from(name), constructor.clone(), exposed)?;
    Ok(Interface {
        prototype: standard.prototype(),
        constructor,
    })
}

/// Build interface `name` inheriting from interface `parent`.
fn derived(
    context: &mut Context,
    name: &str,
    parent: &Interface,
    members: &Members,
) -> JsResult<Interface> {
    interface(
        context,
        name,
        Some(&parent.prototype),
        Some(&parent.constructor),
        members,
    )
}

/// Register every interface, then `window` / `self` / `document`.
pub(crate) fn install(context: &mut Context) -> JsResult<()> {
    use super::node;
    use super::style::{self, HTML_ELEMENT_MEMBERS};
    let event_target = interface(context, "EventTarget", None, None, &NO_MEMBERS)?;
    let node_i = derived(context, "Node", &event_target, &node::NODE_MEMBERS)?;
    let element = derived(context, "Element", &node_i, &node::ELEMENT_MEMBERS)?;
    let character_data = derived(context, "CharacterData", &node_i, &NO_MEMBERS)?;
    let document = derived(context, "Document", &node_i, &node::DOCUMENT_MEMBERS)?;
    let document_fragment = derived(context, "DocumentFragment", &node_i, &NO_MEMBERS)?;
    let html_element = derived(context, "HTMLElement", &element, &HTML_ELEMENT_MEMBERS)?;
    let text = derived(context, "Text", &character_data, &NO_MEMBERS)?;
    let comment = derived(context, "Comment", &character_data, &NO_MEMBERS)?;
    // DOMException.prototype inherits Error.prototype (WebIDL §3.14.1), but
    // the interface object itself is an ordinary function.
    let error = context.intrinsics().constructors().error().prototype();
    let dom_exception = interface(context, "DOMException", Some(&error), None, &DOM_EXCEPTION)?;
    context.insert_data(Protos {
        event_target: event_target.prototype,
        node: node_i.prototype,
        element: element.prototype,
        html_element: html_element.prototype,
        character_data: character_data.prototype,
        text: text.prototype,
        comment: comment.prototype,
        document: document.prototype,
        document_fragment: document_fragment.prototype,
        dom_exception: dom_exception.prototype,
    });

    let global = context.global_object();
    let attr = Attribute::CONFIGURABLE;
    context.register_global_property(js_string!("window"), global.clone(), attr)?;
    context.register_global_property(js_string!("self"), global, attr)?;
    let root = with_state(context, |s| s.host.document().root_index())?;
    let document = wrap(context, root)?;
    context.register_global_property(js_string!("document"), document, attr)?;
    style::install_globals(context)?;
    Ok(())
}

/// The interface prototype matching the node at `index`.
fn prototype_for(context: &mut Context, index: usize) -> JsResult<JsObject> {
    let (kind, html) = with_state(context, |s| {
        let doc = s.host.document();
        let kind = doc.get_node(index).map(|n| n.kind());
        let html = doc.element_namespace_uri(index) == Some(HTML_NS);
        (kind, html)
    })?;
    let p = protos(context);
    Ok(match kind {
        Some(NodeKind::Element) if html => p.html_element.clone(),
        Some(NodeKind::Element) => p.element.clone(),
        Some(NodeKind::Text) => p.text.clone(),
        Some(NodeKind::Comment) => p.comment.clone(),
        Some(NodeKind::Document) => p.document.clone(),
        Some(NodeKind::DocumentFragment) => p.document_fragment.clone(),
        Some(_) => p.node.clone(),
        None => {
            return Err(JsNativeError::typ()
                .with_message("node handle is out of range")
                .into());
        }
    })
}

/// The unique wrapper for arena node `index`, created on first use.
pub(crate) fn wrap(context: &mut Context, index: usize) -> JsResult<JsObject> {
    if let Some(existing) = with_state(context, |s| s.wrappers.get(index).cloned().flatten())? {
        return Ok(existing);
    }
    let proto = prototype_for(context, index)?;
    let object = JsObject::from_proto_and_data(Some(proto), NodeHandle { index });
    with_state(context, |s| {
        if s.wrappers.len() <= index {
            s.wrappers.resize(index + 1, None);
        }
        s.wrappers[index] = Some(object.clone());
    })?;
    Ok(object)
}

/// [`wrap`] for nullable node results.
#[allow(dead_code, reason = "used by members returning `Node?`")]
pub(crate) fn wrap_optional(context: &mut Context, index: Option<usize>) -> JsResult<JsValue> {
    match index {
        Some(index) => Ok(wrap(context, index)?.into()),
        None => Ok(JsValue::null()),
    }
}

fn exception_data<T>(this: &JsValue, f: impl FnOnce(&DomExceptionData) -> T) -> JsResult<T> {
    this.as_object()
        .and_then(|o| o.downcast_ref::<DomExceptionData>().map(|d| f(&d)))
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a DOMException")
                .into()
        })
}

fn dom_exception_name(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    exception_data(this, |d| JsValue::from(JsString::from(d.name.as_str())))
}

fn dom_exception_message(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    exception_data(this, |d| JsValue::from(JsString::from(d.message.as_str())))
}

/// Legacy numeric code for the names this runtime throws (WebIDL §2.8.1 table).
fn dom_exception_code(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    exception_data(this, |d| {
        JsValue::from(match d.name.as_str() {
            "HierarchyRequestError" => 3,
            "InvalidCharacterError" => 5,
            "NotFoundError" => 8,
            "SyntaxError" => 12,
            _ => 0,
        })
    })
}

const DOM_EXCEPTION: Members = Members {
    getters: &[
        ("name", dom_exception_name),
        ("message", dom_exception_message),
        ("code", dom_exception_code),
    ],
    accessors: &[],
    methods: &[],
};

#[cfg(test)]
mod tests;
