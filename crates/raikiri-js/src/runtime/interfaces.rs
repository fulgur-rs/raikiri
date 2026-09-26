//! Interface objects (constructor + prototype pairs) and node wrappers.

use boa_engine::native_function::NativeFunctionPointer;
use boa_engine::object::builtins::JsFunction;
use boa_engine::object::{ConstructorBuilder, FunctionObjectBuilder, JsObject};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Trace,
    js_string,
};
use raikiri_dom::NodeKind;

use super::webidl::{dom_string, with_state};

pub(crate) const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

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
    pub processing_instruction: JsObject,
    pub document: JsObject,
    pub document_fragment: JsObject,
    pub dom_exception: JsObject,
    pub node_list: JsObject,
    pub html_collection: JsObject,
    pub dom_token_list: JsObject,
    pub dom_rect_read_only: JsObject,
    pub dom_rect: JsObject,
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

/// A native getter/setter/method function object with the given `name` and
/// `length` (ECMA-262 §10.2.9: `length` is non-writable, non-enumerable,
/// configurable), built from `native`.
///
/// Shared by [`function`] (interface members, a plain function pointer) and
/// [`closure_function`] (bindings that capture state, such as a `classList`
/// method closing over its element's arena index).
fn function_with_length(
    context: &mut Context,
    name: &str,
    length: usize,
    native: NativeFunction,
) -> JsResult<JsFunction> {
    let function = FunctionObjectBuilder::new(context.realm(), native)
        .name(JsString::from(name))
        .build();
    let length = PropertyDescriptor::builder()
        .value(length)
        .writable(false)
        .enumerable(false)
        .configurable(true);
    function.define_property_or_throw(js_string!("length"), length, context)?;
    Ok(function)
}

/// [`function_with_length`] for a plain function pointer.
pub(crate) fn function(
    context: &mut Context,
    name: &str,
    length: usize,
    f: NativeFunctionPointer,
) -> JsResult<JsFunction> {
    function_with_length(context, name, length, NativeFunction::from_fn_ptr(f))
}

/// [`function_with_length`] for a [`NativeFunction`] built from a closure.
pub(crate) fn closure_function(
    context: &mut Context,
    name: &str,
    length: usize,
    native: NativeFunction,
) -> JsResult<JsFunction> {
    function_with_length(context, name, length, native)
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
/// `members` is a list rather than a single [`Members`] so that an
/// interface combining its own members with one or more WebIDL mixins
/// (e.g. `Element` = its own members + `ParentNode` + `ChildNode` +
/// `NonDocumentTypeChildNode`) can install each `Members` table in turn
/// instead of duplicating them into one combined constant.
///
/// `parent_prototype` becomes the `[[Prototype]]` of the new prototype object
/// and `parent_constructor` the `[[Prototype]]` of the interface object
/// (WebIDL §3.7.1, §3.7.3); either defaults to the ordinary intrinsic.
///
/// `ctor`/`ctor_length` are the interface object's own `[[Call]]`/
/// `[[Construct]]` behavior and WebIDL `length`; every interface without a
/// constructor operation of its own passes [`illegal_constructor`] and `0`
/// (see [`derived`], which always does).
fn interface(
    context: &mut Context,
    name: &str,
    parent_prototype: Option<&JsObject>,
    parent_constructor: Option<&JsObject>,
    members: &[&Members],
    ctor: NativeFunctionPointer,
    ctor_length: usize,
) -> JsResult<Interface> {
    let getters: Vec<_> = members
        .iter()
        .flat_map(|m| m.getters.iter())
        .map(|&(n, f)| Ok((n, function(context, &format!("get {n}"), 0, f)?)))
        .collect::<JsResult<_>>()?;
    let accessors: Vec<_> = members
        .iter()
        .flat_map(|m| m.accessors.iter())
        .map(|&(n, g, s)| {
            let getter = function(context, &format!("get {n}"), 0, g)?;
            let setter = function(context, &format!("set {n}"), 1, s)?;
            Ok((n, getter, setter))
        })
        .collect::<JsResult<_>>()?;
    // WebIDL §3.7.6: operations are writable, enumerable, configurable data
    // properties on the interface prototype.
    let operations: Vec<_> = members
        .iter()
        .flat_map(|m| m.methods.iter())
        .map(|&(n, length, f)| Ok((n, function(context, n, length, f)?)))
        .collect::<JsResult<_>>()?;
    let mut builder = ConstructorBuilder::new(context, NativeFunction::from_fn_ptr(ctor));
    builder.name(name).length(ctor_length).constructor(true);
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
    let operation = Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE;
    for (n, f) in operations {
        builder.property(JsString::from(n), f, operation);
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

/// Build interface `name` inheriting from interface `parent`. Every derived
/// interface here has an illegal constructor -- one with a real constructor
/// operation calls [`interface`] directly instead.
fn derived(
    context: &mut Context,
    name: &str,
    parent: &Interface,
    members: &[&Members],
) -> JsResult<Interface> {
    interface(
        context,
        name,
        Some(&parent.prototype),
        Some(&parent.constructor),
        members,
        illegal_constructor,
        0,
    )
}

/// Register every interface, then `window` / `self` / `document`.
pub(crate) fn install(context: &mut Context) -> JsResult<()> {
    use super::node;
    use super::query;
    use super::style::{self, HTML_ELEMENT_MEMBERS};
    use super::tree;
    use super::{collections, events, geometry, indexed, token_list};
    // A `?` on a call rustfmt wraps across lines leaves the never-taken
    // error-branch region on the closing line, so that line always reports
    // zero hits; binding the call first keeps the `?` on a one-line
    // statement instead (see every `_result` binding below).
    let event_target_result = interface(
        context,
        "EventTarget",
        None,
        None,
        &[&events::EVENT_TARGET_MEMBERS],
        illegal_constructor,
        0,
    );
    let event_target = event_target_result?;
    let node_members = [&node::NODE_MEMBERS, &tree::NODE_TREE_MEMBERS];
    let node_i = derived(context, "Node", &event_target, &node_members)?;
    let element_members = [
        &node::ELEMENT_MEMBERS,
        &tree::PARENT_NODE_MEMBERS,
        &tree::CHILD_NODE_MEMBERS,
        &tree::NON_DOCUMENT_TYPE_CHILD_NODE_MEMBERS,
        &query::PARENT_NODE_QUERY_MEMBERS,
        &query::ELEMENT_QUERY_MEMBERS,
    ];
    let element = derived(context, "Element", &node_i, &element_members)?;
    let character_data_members = [
        &tree::CHARACTER_DATA_MEMBERS,
        &tree::CHILD_NODE_MEMBERS,
        &tree::NON_DOCUMENT_TYPE_CHILD_NODE_MEMBERS,
    ];
    let character_data = derived(context, "CharacterData", &node_i, &character_data_members)?;
    let document_members = [
        &node::DOCUMENT_MEMBERS,
        &tree::PARENT_NODE_MEMBERS,
        &tree::DOCUMENT_CREATE_MEMBERS,
        &query::NON_ELEMENT_PARENT_NODE_MEMBERS,
        &query::PARENT_NODE_QUERY_MEMBERS,
        &query::DOCUMENT_QUERY_MEMBERS,
    ];
    let document = derived(context, "Document", &node_i, &document_members)?;
    let fragment_members = [
        &tree::PARENT_NODE_MEMBERS,
        &query::NON_ELEMENT_PARENT_NODE_MEMBERS,
        &query::PARENT_NODE_QUERY_MEMBERS,
    ];
    let document_fragment = derived(context, "DocumentFragment", &node_i, &fragment_members)?;
    let html_element = derived(context, "HTMLElement", &element, &[&HTML_ELEMENT_MEMBERS])?;
    let text = derived(context, "Text", &character_data, &[&NO_MEMBERS])?;
    let comment = derived(context, "Comment", &character_data, &[&NO_MEMBERS])?;
    let pi_members = [&tree::PROCESSING_INSTRUCTION_MEMBERS];
    // A `?` on a call rustfmt wraps across lines leaves the never-taken
    // error-branch region on the closing line, so that line always reports
    // zero hits; binding the call first keeps the `?` on a one-line
    // statement instead.
    let pi_result = derived(
        context,
        "ProcessingInstruction",
        &character_data,
        &pi_members,
    );
    let pi = pi_result?;
    // DOMException.prototype inherits Error.prototype (WebIDL §3.14.1), but
    // the interface object itself is an ordinary function with a real
    // constructor operation.
    let error = context.intrinsics().constructors().error().prototype();
    let dom_exception_result = interface(
        context,
        "DOMException",
        Some(&error),
        None,
        &[&DOM_EXCEPTION],
        dom_exception_constructor,
        0,
    );
    let dom_exception = dom_exception_result?;
    install_dom_exception_constants(context, &dom_exception)?;
    let node_list_members = [&collections::NODE_LIST_MEMBERS];
    let node_list_result = interface(
        context,
        "NodeList",
        None,
        None,
        &node_list_members,
        illegal_constructor,
        0,
    );
    let node_list = node_list_result?;
    let html_collection_members = [&collections::HTML_COLLECTION_MEMBERS];
    let html_collection_result = interface(
        context,
        "HTMLCollection",
        None,
        None,
        &html_collection_members,
        illegal_constructor,
        0,
    );
    let html_collection = html_collection_result?;
    collections::install_iteration(context, &node_list.prototype, &html_collection.prototype)?;
    let dom_token_list_members = [&token_list::DOM_TOKEN_LIST_MEMBERS];
    let dom_token_list_result = interface(
        context,
        "DOMTokenList",
        None,
        None,
        &dom_token_list_members,
        illegal_constructor,
        0,
    );
    let dom_token_list = dom_token_list_result?;
    token_list::install_iteration(context, &dom_token_list.prototype)?;
    let dom_rect_read_only_members = [&geometry::DOM_RECT_READ_ONLY_MEMBERS];
    let dom_rect_read_only_result = interface(
        context,
        "DOMRectReadOnly",
        None,
        None,
        &dom_rect_read_only_members,
        geometry::dom_rect_read_only_constructor,
        0,
    );
    let dom_rect_read_only = dom_rect_read_only_result?;
    let dom_rect_members = [&geometry::DOM_RECT_MEMBERS];
    let dom_rect_result = interface(
        context,
        "DOMRect",
        Some(&dom_rect_read_only.prototype),
        Some(&dom_rect_read_only.constructor),
        &dom_rect_members,
        geometry::dom_rect_constructor,
        0,
    );
    let dom_rect = dom_rect_result?;
    indexed::install(context)?;
    context.insert_data(Protos {
        event_target: event_target.prototype,
        node: node_i.prototype,
        element: element.prototype,
        html_element: html_element.prototype,
        character_data: character_data.prototype,
        text: text.prototype,
        comment: comment.prototype,
        processing_instruction: pi.prototype,
        document: document.prototype,
        document_fragment: document_fragment.prototype,
        dom_exception: dom_exception.prototype,
        node_list: node_list.prototype,
        html_collection: html_collection.prototype,
        dom_token_list: dom_token_list.prototype,
        dom_rect_read_only: dom_rect_read_only.prototype,
        dom_rect: dom_rect.prototype,
    });

    let global = context.global_object();
    let attr = Attribute::CONFIGURABLE;
    context.register_global_property(js_string!("window"), global.clone(), attr)?;
    context.register_global_property(js_string!("self"), global, attr)?;
    let root = with_state(context, |s| s.host.document().root_index())?;
    let document = wrap(context, root)?;
    context.register_global_property(js_string!("document"), document, attr)?;
    style::install_globals(context)?;
    events::install_globals(context)?;
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
        Some(NodeKind::ProcessingInstruction) => p.processing_instruction.clone(),
        Some(NodeKind::Document) => p.document.clone(),
        Some(NodeKind::DocumentFragment) => p.document_fragment.clone(),
        // cov:ignore: `NodeKind` is `#[non_exhaustive]`; every variant it currently
        // defines already has its own arm above, so this only guards a future one.
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

/// WebIDL §2.8.1's legacy error name -> numeric code table, plus each
/// name's legacy constant identifier. A handful of names don't follow the
/// obvious "insert an underscore before every capital" mapping (`10 =>
/// INUSE_ATTRIBUTE_ERR`, not `IN_USE_...`), so both columns are spelled out
/// rather than derived. Codes 2, 6, and 16 are historical, unused entries
/// the table skips entirely (no name ever throws them, and nothing reads
/// them back from `code`).
const DOM_EXCEPTION_CODES: &[(&str, u16, &str)] = &[
    ("IndexSizeError", 1, "INDEX_SIZE_ERR"),
    ("HierarchyRequestError", 3, "HIERARCHY_REQUEST_ERR"),
    ("WrongDocumentError", 4, "WRONG_DOCUMENT_ERR"),
    ("InvalidCharacterError", 5, "INVALID_CHARACTER_ERR"),
    (
        "NoModificationAllowedError",
        7,
        "NO_MODIFICATION_ALLOWED_ERR",
    ),
    ("NotFoundError", 8, "NOT_FOUND_ERR"),
    ("NotSupportedError", 9, "NOT_SUPPORTED_ERR"),
    ("InUseAttributeError", 10, "INUSE_ATTRIBUTE_ERR"),
    ("InvalidStateError", 11, "INVALID_STATE_ERR"),
    ("SyntaxError", 12, "SYNTAX_ERR"),
    ("InvalidModificationError", 13, "INVALID_MODIFICATION_ERR"),
    ("NamespaceError", 14, "NAMESPACE_ERR"),
    ("InvalidAccessError", 15, "INVALID_ACCESS_ERR"),
    ("TypeMismatchError", 17, "TYPE_MISMATCH_ERR"),
    ("SecurityError", 18, "SECURITY_ERR"),
    ("NetworkError", 19, "NETWORK_ERR"),
    ("AbortError", 20, "ABORT_ERR"),
    ("URLMismatchError", 21, "URL_MISMATCH_ERR"),
    ("QuotaExceededError", 22, "QUOTA_EXCEEDED_ERR"),
    ("TimeoutError", 23, "TIMEOUT_ERR"),
    ("InvalidNodeTypeError", 24, "INVALID_NODE_TYPE_ERR"),
    ("DataCloneError", 25, "DATA_CLONE_ERR"),
];

/// Legacy numeric code for `name` (WebIDL §2.8.1 table); `0` for any other
/// name, including `"Error"` (the `name` default) and every name not in
/// [`DOM_EXCEPTION_CODES`].
fn dom_exception_code(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    exception_data(this, |d| {
        JsValue::from(
            DOM_EXCEPTION_CODES
                .iter()
                .find(|&&(name, ..)| name == d.name)
                .map_or(0, |&(_, code, _)| code),
        )
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

/// `new DOMException(message = "", name = "Error")` (WebIDL §2.8.1).
/// Every constructor built through [`interface`] receives `new.target` as
/// `this` under `[[Construct]]`, or `undefined` under a plain `[[Call]]`
/// (no `new`) -- so `this.is_undefined()` below is exactly WebIDL's "If
/// NewTarget is undefined, throw a TypeError" constructor check.
fn dom_exception_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if this.is_undefined() {
        return Err(JsNativeError::typ()
            .with_message("Constructor DOMException requires 'new'")
            .into());
    }
    let message = optional_dom_string(args, 0, "", context)?;
    let name = optional_dom_string(args, 1, "Error", context)?;
    let proto = protos(context).dom_exception.clone();
    Ok(JsObject::from_proto_and_data(Some(proto), DomExceptionData { name, message }).into())
}

/// `DOMString` conversion for a WebIDL `optional DOMString = default`
/// argument: a genuinely missing trailing argument (or one explicitly
/// passed as `undefined`) takes `default` directly; anything else runs
/// `ToString` as usual, the same as [`super::webidl::dom_string`].
fn optional_dom_string(
    args: &[JsValue],
    i: usize,
    default: &str,
    context: &mut Context,
) -> JsResult<String> {
    match args.get(i) {
        None => Ok(default.to_owned()),
        Some(v) if v.is_undefined() => Ok(default.to_owned()),
        Some(_) => dom_string(args, i, context),
    }
}

/// Define one legacy constant (non-writable, non-configurable, enumerable)
/// on `target`, either `DOMException` or `DOMException.prototype`.
fn define_legacy_constant(
    target: &JsObject,
    name: &str,
    code: u16,
    context: &mut Context,
) -> JsResult<()> {
    let descriptor = PropertyDescriptor::builder()
        .writable(false)
        .enumerable(true)
        .configurable(false)
        .value(code)
        .build();
    target.define_property_or_throw(JsString::from(name), descriptor, context)?;
    Ok(())
}

/// Define every [`DOM_EXCEPTION_CODES`] entry's legacy constant on both
/// `DOMException` (the interface object) and `DOMException.prototype`
/// (WebIDL §3.14: legacy constants live on both).
fn install_dom_exception_constants(
    context: &mut Context,
    dom_exception: &Interface,
) -> JsResult<()> {
    for &(_, code, constant) in DOM_EXCEPTION_CODES {
        define_legacy_constant(&dom_exception.constructor, constant, code, context)?;
        define_legacy_constant(&dom_exception.prototype, constant, code, context)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
