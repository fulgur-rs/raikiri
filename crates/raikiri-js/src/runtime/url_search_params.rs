//! The query-string operations used by WPT variant support scripts.

use boa_engine::object::builtins::JsArray;
use boa_engine::object::{ConstructorBuilder, JsObject};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Trace,
    js_string,
};

use super::interfaces::{function, set_to_string_tag};

#[derive(Debug, Trace, Finalize, JsData)]
struct UrlSearchParamsData {
    #[unsafe_ignore_trace]
    entries: Vec<(String, String)>,
}

fn params_entries(this: &JsValue) -> JsResult<Vec<(String, String)>> {
    this.as_object()
        .and_then(|object| {
            object
                .downcast_ref::<UrlSearchParamsData>()
                .map(|data| data.entries.clone())
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
    let init = match args.first() {
        Some(value) => value.to_string(context)?.to_std_string_escaped(),
        None => String::new(),
    };
    let query = init.strip_prefix('?').unwrap_or(&init);
    let entries = url::form_urlencoded::parse(query.as_bytes())
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    let object = JsObject::from_proto_and_data(Some(prototype), UrlSearchParamsData { entries });
    Ok(object.into())
}

fn url_search_params_has(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = required_name(args, context)?;
    let entries = params_entries(this)?;
    Ok(JsValue::from(entries.iter().any(|(key, _)| key == &name)))
}

fn url_search_params_get(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = required_name(args, context)?;
    let entries = params_entries(this)?;
    Ok(entries
        .iter()
        .find(|(key, _)| key == &name)
        .map_or_else(JsValue::null, |(_, value)| {
            JsString::from(value.as_str()).into()
        }))
}

fn url_search_params_get_all(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = required_name(args, context)?;
    let entries = params_entries(this)?;
    let values = entries
        .iter()
        .filter(|(key, _)| key == &name)
        .map(|(_, value)| JsValue::from(JsString::from(value.as_str())));
    Ok(JsArray::from_iter(values, context).into())
}

pub(crate) fn install(context: &mut Context) -> JsResult<()> {
    let has = function(context, "has", 1, url_search_params_has)?;
    let get = function(context, "get", 1, url_search_params_get)?;
    let get_all = function(context, "getAll", 1, url_search_params_get_all)?;
    let mut builder = ConstructorBuilder::new(
        context,
        NativeFunction::from_fn_ptr(url_search_params_constructor),
    );
    builder.name("URLSearchParams").length(1).constructor(true);
    let operation = Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE;
    builder.property(js_string!("has"), has, operation);
    builder.property(js_string!("get"), get, operation);
    builder.property(js_string!("getAll"), get_all, operation);
    let standard = builder.build();
    let prototype = standard.prototype();
    let constructor = standard.constructor();
    set_to_string_tag(&prototype, "URLSearchParams", context)?;
    context.register_global_property(
        js_string!("URLSearchParams"),
        constructor,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;
    Ok(())
}
