//! The query-string operations used by WPT variant support scripts.

use boa_engine::object::builtins::JsArray;
use boa_engine::object::{ConstructorBuilder, JsObject};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Trace,
    js_string,
};

use super::interfaces::{function, guarded_native, set_to_string_tag};

#[derive(Debug, Trace, Finalize, JsData)]
struct UrlSearchParamsData {
    #[unsafe_ignore_trace]
    entries: Vec<(String, String)>,
}

fn params_entries(this: &JsValue) -> JsResult<Vec<(String, String)>> {
    this.as_object()
        .and_then(|object| {
            object.downcast_ref::<UrlSearchParamsData>().map(|data| {
                #[cfg(any(test, feature = "url-params-diagnostics"))]
                crate::url_params_diagnostics::snapshot(data.entries.len());
                data.entries.clone()
            })
        })
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a URLSearchParams")
                .into()
        })
}

/// Read entries without taking a snapshot, returning an owned result.
/// Convert names before calling this helper; build context-owned arrays
/// after it returns so no context operation runs under the data borrow.
fn with_params<R>(this: &JsValue, read: impl FnOnce(&[(String, String)]) -> R) -> JsResult<R> {
    this.as_object()
        .and_then(|object| {
            object
                .downcast_ref::<UrlSearchParamsData>()
                .map(|data| read(&data.entries))
        })
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a URLSearchParams")
                .into()
        })
}

fn required_name(args: &[JsValue], context: &mut Context) -> JsResult<String> {
    let value = args
        .first()
        .ok_or_else(|| JsNativeError::typ().with_message("name is required"))?;
    Ok(value.to_string(context)?.to_std_string_escaped())
}

fn url_search_params_constructor(
    new_target: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let new_target = new_target.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("URLSearchParams constructor requires 'new'")
    })?;
    let prototype = new_target
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("constructor prototype is not an object")
        })?;
    super::webidl::guard_active(context)?;
    let init = match args.first() {
        Some(_) => super::webidl::dom_string(args, 0, context)?,
        None => String::new(),
    };
    Ok(params_object(prototype, &init).into())
}

/// A `URLSearchParams` with `prototype`, initialized from the query string
/// `init` (a leading `?` is skipped).
pub(crate) fn params_object(prototype: JsObject, init: &str) -> JsObject {
    let query = init.strip_prefix('?').unwrap_or(init);
    let entries = url::form_urlencoded::parse(query.as_bytes())
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    JsObject::from_proto_and_data(Some(prototype), UrlSearchParamsData { entries })
}

fn url_search_params_has(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = required_name(args, context)?;
    with_params(this, |entries| {
        JsValue::from(entries.iter().any(|(key, _)| key == &name))
    })
}

fn url_search_params_get(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = required_name(args, context)?;
    with_params(this, |entries| {
        entries
            .iter()
            .find(|(key, _)| key == &name)
            .map_or_else(JsValue::null, |(_, value)| {
                JsString::from(value.as_str()).into()
            })
    })
}

fn url_search_params_get_all(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = required_name(args, context)?;
    let values = with_params(this, |entries| {
        entries
            .iter()
            .filter(|(key, _)| key == &name)
            .map(|(_, value)| JsValue::from(JsString::from(value.as_str())))
            .collect::<Vec<_>>()
    })?;
    Ok(JsArray::from_iter(values, context).into())
}

/// `URLSearchParams.prototype.entries` and `@@iterator`: an iterator over
/// `[name, value]` pairs, here an array iterator over a snapshot of them.
fn url_search_params_entries(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let entries = params_entries(this)?;
    let pairs: Vec<JsValue> = entries
        .iter()
        .map(|(name, value)| {
            let pair = [
                JsValue::from(JsString::from(name.as_str())),
                JsValue::from(JsString::from(value.as_str())),
            ];
            JsArray::from_iter(pair, context).into()
        })
        .collect();
    JsArray::from_iter(pairs, context).values(context)
}

/// Installs `URLSearchParams` and returns its prototype.
pub(crate) fn install(context: &mut Context) -> JsResult<JsObject> {
    let has = function(context, "has", 1, url_search_params_has)?;
    let entries = function(context, "entries", 0, url_search_params_entries)?;
    let get = function(context, "get", 1, url_search_params_get)?;
    let get_all = function(context, "getAll", 1, url_search_params_get_all)?;
    let mut builder = ConstructorBuilder::new(
        context,
        guarded_native(NativeFunction::from_fn_ptr(url_search_params_constructor)),
    );
    builder.name("URLSearchParams").length(1).constructor(true);
    let operation = Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE;
    builder.property(js_string!("has"), has, operation);
    builder.property(js_string!("get"), get, operation);
    builder.property(js_string!("getAll"), get_all, operation);
    builder.property(js_string!("entries"), entries.clone(), operation);
    builder.property(boa_engine::JsSymbol::iterator(), entries, operation);
    let standard = builder.build();
    let prototype = standard.prototype();
    let constructor = standard.constructor();
    set_to_string_tag(&prototype, "URLSearchParams", context)?;
    context.register_global_property(
        js_string!("URLSearchParams"),
        constructor,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;
    Ok(prototype)
}

#[cfg(test)]
mod tests;
