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
//! object.
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
    let targets = JsWeakMap::new(context);
    context.insert_data(IndexedIntrinsics {
        targets,
        reflect_get,
        reflect_get_own_property_descriptor,
        reflect_define_property,
        reflect_delete_property,
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
    let reflect_get = intrinsics(context)?.reflect_get.clone();
    call(&reflect_get, args, context)
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
        let descriptor = JsObject::with_object_proto(context.intrinsics());
        descriptor.create_data_property_or_throw(js_string!("value"), value, context)?;
        descriptor.create_data_property_or_throw(js_string!("writable"), false, context)?;
        descriptor.create_data_property_or_throw(js_string!("enumerable"), true, context)?;
        descriptor.create_data_property_or_throw(js_string!("configurable"), true, context)?;
        return Ok(descriptor.into());
    }
    let reflect = intrinsics(context)?
        .reflect_get_own_property_descriptor
        .clone();
    call(&reflect, args, context)
}

/// `defineProperty` trap (`« target, key, descriptor »`): array indices are
/// read-only (no indexed setter), everything else is defined on the target.
fn define_property_trap(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if array_index(args, context)?.is_some() {
        return Ok(false.into());
    }
    let reflect = intrinsics(context)?.reflect_define_property.clone();
    call(&reflect, args, context)
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
    let reflect = intrinsics(context)?.reflect_delete_property.clone();
    call(&reflect, args, context)
}

/// `preventExtensions` trap: always refuses (see the module docs).
fn prevent_extensions_trap(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(false.into())
}
