//! Objects with WebIDL indexed properties (`coll[0]`, `coll.length`),
//! built as a `Proxy` over a native target.
//!
//! WebIDL §3.9 ("legacy platform objects") gives an interface with an
//! indexed property getter exotic `[[GetOwnProperty]]`,
//! `[[DefineOwnProperty]]`, `[[Delete]]`, `[[PreventExtensions]]`, and
//! `[[OwnPropertyKeys]]` behavior. Boa has no hook for custom exotic
//! objects outside the engine, so [`indexed_object`] builds the same
//! observable behavior from a `Proxy` whose traps consult an
//! [`IndexedSource`] every time, which is what makes a collection *live*.
//!
//! # Proxy invariants
//!
//! Supported indices are reported as own data properties that are
//! `{ writable: false, enumerable: true, configurable: true }` and never
//! actually exist on the target. ECMAScript only lets a `getOwnPropertyDescriptor`
//! trap report a property the target lacks when the descriptor is
//! configurable **and** the target is extensible, and only lets an `ownKeys`
//! trap add keys the target lacks while the target is extensible. So:
//!
//! - every index descriptor is configurable, and
//! - the `preventExtensions` trap always returns `false` (WebIDL's
//!   `[[PreventExtensions]]` for legacy platform objects does the same), so
//!   the target can never become non-extensible;
//! - the `defineProperty` trap refuses every array-index key, so the target
//!   never gains an own index property that could disagree with the source.
//!
//! Every other key is forwarded to the target with the original receiver,
//! so expandos, symbols, and prototype members behave as on an ordinary
//! object. `set` is forwarded as `Reflect.set(target, key, value, receiver)`
//! for every key: with the proxy as receiver, an index write reaches this
//! proxy's own `getOwnPropertyDescriptor` / `defineProperty` traps, so it
//! fails exactly as `[[DefineOwnProperty]]` does.
//!
//! # Every trap is defined
//!
//! A proxy looks each trap up on its handler with an ordinary `[[Get]]`,
//! which walks the handler's prototype chain, and the handler
//! `JsProxyBuilder` creates inherits from `Object.prototype`. A trap left
//! undefined could therefore be supplied by a script that adds, say,
//! `Object.prototype.getPrototypeOf`, and that function would receive the
//! native target. So every trap that can fire on a non-callable target is
//! installed, the ones with nothing to add as plain forwards to the matching
//! `Reflect` function captured by [`install`]. For the same reason the
//! descriptor objects exchanged with `Reflect` (the one
//! `getOwnPropertyDescriptor` returns, and the one `defineProperty` passes
//! on) are null-prototype copies, so that converting them to a property
//! descriptor cannot pick up a polluted `Object.prototype.get` / `set`.
//!
//! # Brand checks
//!
//! A prototype member called through the proxy sees the proxy as `this`,
//! and Boa exposes no public way to read a proxy's target. Every proxy made
//! here is recorded in a private `WeakMap` (proxy -> target) that no script
//! can reach; [`this_indexed`] resolves `this` through that map and then
//! downcasts the target to the expected source type. A script-made
//! `new Proxy(collection, {})` is not in the map and fails the check, as in
//! browsers.

use std::rc::Rc;

use boa_engine::object::JsObject;
use boa_engine::object::builtins::{JsArray, JsProxyBuilder, JsWeakMap};
use boa_engine::property::PropertyKey;
use boa_engine::{
    Context, Finalize, JsData, JsError, JsNativeError, JsResult, JsString, JsValue, Trace,
    js_string,
};

/// What an indexed object enumerates. Implementations must not own
/// garbage-collected values: the target's native data is not traced.
pub(crate) trait IndexedSource: 'static {
    /// The number of supported property indices right now.
    fn length(&self, context: &mut Context) -> JsResult<usize>;

    /// The value at `index`, or `None` when `index` is not supported.
    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>>;
}

/// Native data of an indexed object's proxy target.
#[derive(Trace, Finalize, JsData)]
struct Indexed<S: IndexedSource> {
    #[unsafe_ignore_trace]
    source: Rc<S>,
}

/// Engine functions captured before any script runs, so that later
/// changes to the global `Reflect` object cannot affect the traps, plus the
/// private proxy -> target map used by [`this_indexed`].
struct IndexedIntrinsics {
    targets: JsWeakMap,
    reflect_get: JsObject,
    reflect_get_own_property_descriptor: JsObject,
    reflect_define_property: JsObject,
    reflect_delete_property: JsObject,
    reflect_set: JsObject,
    reflect_get_prototype_of: JsObject,
    reflect_set_prototype_of: JsObject,
    reflect_is_extensible: JsObject,
}

/// Capture the intrinsics [`indexed_object`] needs. Must run before any
/// script is evaluated.
pub(crate) fn install(context: &mut Context) -> JsResult<()> {
    let reflect = context.intrinsics().objects().reflect();
    let mut function = |name| -> JsResult<JsObject> {
        let value = reflect.get(name, context)?;
        // cov:ignore: every name looked up here is a built-in `Reflect`
        // function, and no script has run yet to replace it.
        value.as_object().ok_or_else(target_error)
    };
    let reflect_get = function(js_string!("get"))?;
    let reflect_get_own_property_descriptor = function(js_string!("getOwnPropertyDescriptor"))?;
    let reflect_define_property = function(js_string!("defineProperty"))?;
    let reflect_delete_property = function(js_string!("deleteProperty"))?;
    let reflect_set = function(js_string!("set"))?;
    let reflect_get_prototype_of = function(js_string!("getPrototypeOf"))?;
    let reflect_set_prototype_of = function(js_string!("setPrototypeOf"))?;
    let reflect_is_extensible = function(js_string!("isExtensible"))?;
    let targets = JsWeakMap::new(context);
    context.insert_data(IndexedIntrinsics {
        targets,
        reflect_get,
        reflect_get_own_property_descriptor,
        reflect_define_property,
        reflect_delete_property,
        reflect_set,
        reflect_get_prototype_of,
        reflect_set_prototype_of,
        reflect_is_extensible,
    });
    Ok(())
}

fn intrinsics(context: &Context) -> JsResult<&IndexedIntrinsics> {
    // cov:ignore: `install` runs while the runtime is created, before any
    // binding can be called.
    context
        .get_data::<IndexedIntrinsics>()
        .ok_or_else(target_error)
}

// cov:ignore: traps are only ever installed on targets built by
// `indexed_object`, and `intrinsics` always finds the data `install` stored,
// so no script can reach this; it exists so a future misuse throws instead
// of panicking.
fn target_error() -> JsError {
    JsNativeError::typ()
        .with_message("indexed object proxy called on an unexpected target")
        .into()
}

/// A new indexed object whose `[[Prototype]]` is `prototype` and whose
/// indices come from `source`.
pub(crate) fn indexed_object<S: IndexedSource>(
    context: &mut Context,
    prototype: JsObject,
    source: S,
) -> JsResult<JsObject> {
    let target = JsObject::from_proto_and_data(
        Some(prototype),
        Indexed {
            source: Rc::new(source),
        },
    );
    let proxy: JsObject = JsProxyBuilder::new(target.clone())
        .get(get_trap::<S>)
        .has(has_trap::<S>)
        .own_keys(own_keys_trap::<S>)
        .get_own_property_descriptor(get_own_property_descriptor_trap::<S>)
        .define_property(define_property_trap)
        .delete_property(delete_property_trap::<S>)
        .prevent_extensions(prevent_extensions_trap)
        .set(set_trap)
        .get_prototype_of(get_prototype_of_trap)
        .set_prototype_of(set_prototype_of_trap)
        .is_extensible(is_extensible_trap)
        .build(context)?
        .into();
    let targets = intrinsics(context)?.targets.clone();
    targets.set(&proxy, target.into(), context)?;
    Ok(proxy)
}

/// Brand check for members of the interface whose objects carry an `S`
/// source: `this` must be an indexed object made with an `S`, otherwise a
/// `TypeError` naming `interface` is thrown.
pub(crate) fn this_indexed<S: IndexedSource>(
    this: &JsValue,
    context: &mut Context,
    interface: &str,
) -> JsResult<Rc<S>> {
    let error = || -> JsError {
        JsNativeError::typ()
            .with_message(format!("'this' is not a {interface}"))
            .into()
    };
    let object = this.as_object().ok_or_else(error)?;
    let targets = intrinsics(context)?.targets.clone();
    let target = targets.get(&object, context)?;
    target
        .as_object()
        .and_then(|t| t.downcast_ref::<Indexed<S>>().map(|d| d.source.clone()))
        .ok_or_else(error)
}

/// The target object (trap argument 0) and its source.
fn trap_target<S: IndexedSource>(args: &[JsValue]) -> JsResult<(JsObject, Rc<S>)> {
    let target = args
        .first()
        .and_then(JsValue::as_object)
        .ok_or_else(target_error)?;
    let source = target
        .downcast_ref::<Indexed<S>>()
        .map(|d| d.source.clone())
        .ok_or_else(target_error)?;
    Ok((target, source))
}

/// The array index named by trap argument 1, if it is one. Boa normalizes
/// canonical array-index strings ("0", "17", but not "01", "-0", or "1.0")
/// to [`PropertyKey::Index`], so that is the only classification needed.
fn array_index(args: &[JsValue], context: &mut Context) -> JsResult<Option<usize>> {
    let key = args.get(1).cloned().unwrap_or_default();
    Ok(match key.to_property_key(context)? {
        PropertyKey::Index(index) => Some(index.get() as usize),
        _ => None,
    })
}

fn call(function: &JsObject, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    function.call(&JsValue::undefined(), args, context)
}

/// Call the captured `Reflect` function `pick` selects with the trap's own
/// arguments.
fn forward(
    pick: fn(&IndexedIntrinsics) -> &JsObject,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let function = pick(intrinsics(context)?).clone();
    call(&function, args, context)
}

/// A null-prototype copy of the descriptor object `value` (or `undefined`
/// as is), so that reading it back as a descriptor sees only its own
/// fields.
fn detached_descriptor(value: JsValue, context: &mut Context) -> JsResult<JsValue> {
    let Some(source) = value.as_object() else {
        return Ok(value);
    };
    let descriptor = JsObject::with_null_proto();
    for field in [
        "value",
        "writable",
        "get",
        "set",
        "enumerable",
        "configurable",
    ] {
        let field = JsString::from(field);
        if source.has_own_property(field.clone(), context)? {
            let value = source.get(field.clone(), context)?;
            descriptor.create_data_property_or_throw(field, value, context)?;
        }
    }
    Ok(descriptor.into())
}

/// `get` trap (`« target, key, receiver »`): a supported index reads the
/// source; anything else is `Reflect.get(target, key, receiver)`.
fn get_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (_, source) = trap_target::<S>(args)?;
    if let Some(index) = array_index(args, context)?
        && let Some(value) = source.item(index, context)?
    {
        return Ok(value);
    }
    forward(|i| &i.reflect_get, args, context)
}

/// `has` trap (`« target, key »`).
fn has_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (target, source) = trap_target::<S>(args)?;
    if let Some(index) = array_index(args, context)?
        && index < source.length(context)?
    {
        return Ok(true.into());
    }
    let key = args.get(1).cloned().unwrap_or_default();
    let key = key.to_property_key(context)?;
    Ok(target.has_property(key, context)?.into())
}

/// `ownKeys` trap (`« target »`): the supported indices in ascending order,
/// then the target's own keys (strings, then symbols, as the target orders
/// them).
fn own_keys_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (target, source) = trap_target::<S>(args)?;
    let length = source.length(context)?;
    let mut keys: Vec<JsValue> = (0..length)
        .map(|i| JsValue::from(JsString::from(i.to_string())))
        .collect();
    keys.extend(
        target
            .own_property_keys(context)?
            .into_iter()
            .map(JsValue::from),
    );
    Ok(JsArray::from_iter(keys, context).into())
}

/// `getOwnPropertyDescriptor` trap (`« target, key »`).
fn get_own_property_descriptor_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (_, source) = trap_target::<S>(args)?;
    if let Some(index) = array_index(args, context)?
        && let Some(value) = source.item(index, context)?
    {
        let descriptor = JsObject::with_null_proto();
        descriptor.create_data_property_or_throw(js_string!("value"), value, context)?;
        descriptor.create_data_property_or_throw(js_string!("writable"), false, context)?;
        descriptor.create_data_property_or_throw(js_string!("enumerable"), true, context)?;
        descriptor.create_data_property_or_throw(js_string!("configurable"), true, context)?;
        return Ok(descriptor.into());
    }
    let own = forward(|i| &i.reflect_get_own_property_descriptor, args, context)?;
    detached_descriptor(own, context)
}

/// `defineProperty` trap (`« target, key, descriptor »`): array indices are
/// read-only (no indexed setter), everything else is defined on the target.
fn define_property_trap(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if array_index(args, context)?.is_some() {
        return Ok(false.into());
    }
    // The engine builds the descriptor argument with `Object.prototype` as
    // its prototype; detach it before `Reflect.defineProperty` reads it back.
    let target = args.first().cloned().unwrap_or_default();
    let key = args.get(1).cloned().unwrap_or_default();
    let descriptor = detached_descriptor(args.get(2).cloned().unwrap_or_default(), context)?;
    forward(
        |i| &i.reflect_define_property,
        &[target, key, descriptor],
        context,
    )
}

/// `deleteProperty` trap (`« target, key »`): a supported index cannot be
/// deleted; anything else is deleted from the target.
fn delete_property_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (_, source) = trap_target::<S>(args)?;
    if let Some(index) = array_index(args, context)?
        && index < source.length(context)?
    {
        return Ok(false.into());
    }
    forward(|i| &i.reflect_delete_property, args, context)
}

/// `set` trap (`« target, key, value, receiver »`), forwarded for every key
/// (see the module docs for how index writes fail).
fn set_trap(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    forward(|i| &i.reflect_set, args, context)
}

/// `getPrototypeOf` trap (`« target »`).
fn get_prototype_of_trap(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    forward(|i| &i.reflect_get_prototype_of, args, context)
}

/// `setPrototypeOf` trap (`« target, prototype »`).
fn set_prototype_of_trap(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    forward(|i| &i.reflect_set_prototype_of, args, context)
}

/// `isExtensible` trap (`« target »`).
fn is_extensible_trap(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    forward(|i| &i.reflect_is_extensible, args, context)
}

/// `preventExtensions` trap: always refuses (see the module docs).
fn prevent_extensions_trap(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(false.into())
}
