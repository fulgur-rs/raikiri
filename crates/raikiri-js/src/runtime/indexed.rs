//! Objects with WebIDL indexed properties (`coll[0]`, `coll.length`),
//! plus optional named properties (`coll.someId`), built as a `Proxy`
//! over a native target.
//!
//! WebIDL §3.9 ("legacy platform objects") gives an interface with an
//! indexed property getter exotic `[[GetOwnProperty]]`,
//! `[[DefineOwnProperty]]`, `[[Delete]]`, `[[PreventExtensions]]`, and
//! `[[OwnPropertyKeys]]` behavior, extended here with the named-property
//! side of the same section (`LegacyPlatformObjectGetOwnProperty` with
//! `ignoreNamedProps = false`, the named-property visibility algorithm,
//! and the named branches of `[[DefineOwnProperty]]` / `[[Delete]]` /
//! `[[OwnPropertyKeys]]`). Boa has no hook for custom exotic
//! objects outside the engine, so [`indexed_object`] builds the same
//! observable behavior from a `Proxy` whose traps consult an
//! [`IndexedSource`] every time, which is what makes a collection *live*.
//!
//! # Proxy invariants
//!
//! Supported indices are reported as own data properties that are
//! `{ writable: false, enumerable: true, configurable: true }` and never
//! actually exist on the target. Visible named properties are reported the
//! same way, except `enumerable` follows [`IndexedSource::named_enumerable`]
//! (`false` for `HTMLCollection`'s `[LegacyUnenumerableNamedProperties]`).
//! ECMAScript only lets a `getOwnPropertyDescriptor`
//! trap report a property the target lacks when the descriptor is
//! configurable **and** the target is extensible, and only lets an `ownKeys`
//! trap add keys the target lacks while the target is extensible. So:
//!
//! - every index and named descriptor is configurable, and
//! - the `preventExtensions` trap always returns `false` (WebIDL's
//!   `[[PreventExtensions]]` for legacy platform objects does the same), so
//!   the target can never become non-extensible;
//! - the `defineProperty` trap refuses every array-index key, and every
//!   supported named-property key without an own property (no source here
//!   implements a named setter), so the target never gains an own property
//!   that could disagree with the source.
//!
//! Every other key is forwarded to the target with the original receiver,
//! so expandos, symbols, and prototype members behave as on an ordinary
//! object. `set` is forwarded as `Reflect.set(target, key, value, receiver)`
//! for every key: with the proxy as receiver, an index or visible-named
//! write reaches this proxy's own `getOwnPropertyDescriptor` /
//! `defineProperty` traps, so it fails exactly as `[[DefineOwnProperty]]`
//! does.
//!
//! # What is not implemented
//!
//! All sources here share the same narrow shape, which keeps the traps
//! spec-faithful without carrying the full WebIDL generality:
//!
//! - no indexed or named setter, and no named deleter;
//! - no `[LegacyOverrideBuiltIns]` (named properties never shadow own or
//!   prototype properties) and no `[Global]` (the named
//!   `[[DefineOwnProperty]]` / `[[Delete]]` branches for global objects do
//!   not apply);
//! - no unforgeable property names;
//! - no named-properties object on any prototype chain (only `[Global]`
//!   interfaces have one, and none of these objects is global).
//!
//! A source that needs any of those (for example a future
//! `CSSStyleDeclaration`-style object with a named setter) extends
//! [`IndexedSource`] with new hook methods rather than branching here.
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
///
/// The named-property hooks default to "no named properties" (`NodeList`,
/// `DOMTokenList`, and `CSSStyleDeclaration` keep the defaults today; only
/// `HTMLCollection` overrides them), so the same building block can serve
/// `CSSStyleDeclaration`-style objects later by overriding the same two
/// methods.
pub(crate) trait IndexedSource: 'static {
    /// The number of supported property indices right now.
    fn length(&self, context: &mut Context) -> JsResult<usize>;

    /// The value at `index`, or `None` when `index` is not supported.
    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>>;

    /// The supported property names right now, in specification order
    /// (DOM §4.2.10.2 for `HTMLCollection`: tree order, `id` then `name`
    /// per element, deduplicated). Empty when the interface supports no
    /// named properties.
    fn supported_property_names(&self, context: &mut Context) -> JsResult<Vec<String>> {
        let _ = context;
        Ok(Vec::new())
    }

    /// The named getter's value for `name`, or `None` when `name` is not a
    /// supported property name (including the empty string, which never
    /// matches per DOM §4.2.10.2).
    fn named_property(&self, name: &str, context: &mut Context) -> JsResult<Option<JsValue>> {
        let _ = (name, context);
        Ok(None)
    }

    /// Whether named properties are enumerable (`false` for interfaces with
    /// `[LegacyUnenumerableNamedProperties]`, such as `HTMLCollection`).
    fn named_enumerable(&self) -> bool {
        false
    }

    /// Whether the interface has `[LegacyOverrideBuiltIns]` (named
    /// properties shadow own and prototype properties). None of the current
    /// sources does.
    fn legacy_override_built_ins(&self) -> bool {
        false
    }
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
        .define_property(define_property_trap::<S>)
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

/// The string name in trap argument 1, if it is one. Array indices arrive
/// as `Index` and symbols as `Symbol`, so only `String` counts: named
/// properties are strings by definition, and WebIDL ignores named
/// properties for array indices entirely.
fn string_name(args: &[JsValue], context: &mut Context) -> JsResult<Option<String>> {
    let key = args.get(1).cloned().unwrap_or_default();
    Ok(match key.to_property_key(context)? {
        PropertyKey::String(name) => Some(name.to_std_string_escaped()),
        _ => None,
    })
}

/// Whether `name` is visible on `target` per the named-property visibility
/// algorithm (WebIDL §3.9.7), assuming `name` is already known to be a
/// supported property name (the caller checked via `named_property` or the
/// supported-names list, matching the algorithm's first step).
///
/// The caller guarantees `name` is not an array index that is also a
/// supported index: `own_keys_trap` filters those before calling (they are
/// already listed as indices, and the indexed own property makes the named
/// one invisible), while the other traps only call this for `String` keys,
/// which Boa never classifies as `Index`.
fn is_named_visible<S: IndexedSource>(
    target: &JsObject,
    source: &S,
    name: &str,
    context: &mut Context,
) -> JsResult<bool> {
    let key = JsString::from(name);
    if target.has_own_property(key.clone(), context)? {
        return Ok(false);
    }
    if source.legacy_override_built_ins() {
        return Ok(true);
    }
    let mut prototype = target.prototype();
    while let Some(object) = prototype {
        if object.has_own_property(key.clone(), context)? {
            return Ok(false);
        }
        prototype = object.prototype();
    }
    Ok(true)
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

/// `get` trap: a supported index reads the source; a visible named
/// property reads the named getter; anything else is
/// `Reflect.get(target, key, receiver)`. Array indices never consult named
/// properties, matching `LegacyPlatformObjectGetOwnProperty` setting
/// `ignoreNamedProps` for them.
fn get_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (target, source) = trap_target::<S>(args)?;
    if let Some(index) = array_index(args, context)? {
        if let Some(value) = source.item(index, context)? {
            return Ok(value);
        }
        return forward(|i| &i.reflect_get, args, context);
    }
    if let Some(name) = string_name(args, context)?
        && let Some(value) = source.named_property(&name, context)?
        && is_named_visible(&target, source.as_ref(), &name, context)?
    {
        return Ok(value);
    }
    forward(|i| &i.reflect_get, args, context)
}

/// `has` trap: supported indices and visible named properties are
/// present; anything else follows the target (own plus prototype).
/// Array indices never consult named properties.
fn has_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (target, source) = trap_target::<S>(args)?;
    if let Some(index) = array_index(args, context)? {
        if index < source.length(context)? {
            return Ok(true.into());
        }
        let key = args.get(1).cloned().unwrap_or_default();
        let key = key.to_property_key(context)?;
        return Ok(target.has_property(key, context)?.into());
    }
    let key = args.get(1).cloned().unwrap_or_default();
    let key = key.to_property_key(context)?;
    if target.has_property(key, context)? {
        return Ok(true.into());
    }
    if let Some(name) = string_name(args, context)?
        && let Some(_) = source.named_property(&name, context)?
        && is_named_visible(&target, source.as_ref(), &name, context)?
    {
        return Ok(true.into());
    }
    Ok(false.into())
}

/// `ownKeys` trap: the supported indices in ascending order, then the
/// visible named properties in supported-names order, then the target's
/// own keys (strings, then symbols, as the target orders them).
/// Supported names that are also supported indices are skipped here (they
/// are already listed as indices, and the indexed own property makes the
/// named one invisible).
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
    for name in source.supported_property_names(context)? {
        if let PropertyKey::Index(index) = PropertyKey::from(JsString::from(name.as_str()))
            && (index.get() as usize) < length
        {
            continue;
        }
        if is_named_visible(&target, source.as_ref(), &name, context)? {
            keys.push(JsValue::from(JsString::from(name)));
        }
    }
    keys.extend(
        target
            .own_property_keys(context)?
            .into_iter()
            .map(JsValue::from),
    );
    Ok(JsArray::from_iter(keys, context).into())
}

/// `getOwnPropertyDescriptor` trap: supported indices report
/// `{ writable: false, enumerable: true, configurable: true }`; visible
/// named properties report `{ writable: false, enumerable:
/// named_enumerable, configurable: true }` (no source implements a named
/// setter); array indices never consult named properties; anything else
/// forwards to the target with a detached copy.
fn get_own_property_descriptor_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (target, source) = trap_target::<S>(args)?;
    if let Some(index) = array_index(args, context)? {
        if let Some(value) = source.item(index, context)? {
            let descriptor = JsObject::with_null_proto();
            descriptor.create_data_property_or_throw(js_string!("value"), value, context)?;
            descriptor.create_data_property_or_throw(js_string!("writable"), false, context)?;
            descriptor.create_data_property_or_throw(js_string!("enumerable"), true, context)?;
            descriptor.create_data_property_or_throw(js_string!("configurable"), true, context)?;
            return Ok(descriptor.into());
        }
        let own = forward(|i| &i.reflect_get_own_property_descriptor, args, context)?;
        return detached_descriptor(own, context);
    }
    if let Some(name) = string_name(args, context)?
        && let Some(value) = source.named_property(&name, context)?
        && is_named_visible(&target, source.as_ref(), &name, context)?
    {
        let enumerable = source.named_enumerable();
        let descriptor = JsObject::with_null_proto();
        descriptor.create_data_property_or_throw(js_string!("value"), value, context)?;
        descriptor.create_data_property_or_throw(js_string!("writable"), false, context)?;
        descriptor.create_data_property_or_throw(js_string!("enumerable"), enumerable, context)?;
        descriptor.create_data_property_or_throw(js_string!("configurable"), true, context)?;
        return Ok(descriptor.into());
    }
    let own = forward(|i| &i.reflect_get_own_property_descriptor, args, context)?;
    detached_descriptor(own, context)
}

/// `defineProperty` trap: array indices are read-only (no indexed
/// setter, so every array-index key refuses); a supported named property
/// without an own property is read-only too (no named setter), so defining
/// it refuses as well; redefining an existing own property and defining an
/// unsupported name both fall through to the target. Symbols always fall
/// through.
fn define_property_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if array_index(args, context)?.is_some() {
        return Ok(false.into());
    }
    if let Some(name) = string_name(args, context)? {
        let (target, source) = trap_target::<S>(args)?;
        if source.named_property(&name, context)?.is_some()
            && !target.has_own_property(JsString::from(name.as_str()), context)?
            && !source.legacy_override_built_ins()
        {
            return Ok(false.into());
        }
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

/// `deleteProperty` trap: a supported index cannot be deleted; a visible
/// named property cannot be deleted either (no named deleter); array
/// indices never consult named properties; anything else is deleted from
/// the target.
fn delete_property_trap<S: IndexedSource>(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (target, source) = trap_target::<S>(args)?;
    if let Some(index) = array_index(args, context)? {
        if index < source.length(context)? {
            return Ok(false.into());
        }
        return forward(|i| &i.reflect_delete_property, args, context);
    }
    if let Some(name) = string_name(args, context)?
        && source.named_property(&name, context)?.is_some()
        && is_named_visible(&target, source.as_ref(), &name, context)?
    {
        return Ok(false.into());
    }
    forward(|i| &i.reflect_delete_property, args, context)
}

/// `set` trap, forwarded for every key (see the module docs for how
/// index and visible-named writes fail through the `defineProperty` trap).
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
