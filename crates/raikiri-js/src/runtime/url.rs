//! The `URL` interface (WHATWG URL Standard §6.1), read-only: scripts build
//! URLs and read their components, but nothing here navigates or fetches.

use boa_engine::object::{ConstructorBuilder, JsObject};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Trace,
    js_string,
};
use url::Url;

use super::interfaces::{function, set_to_string_tag};

#[derive(Debug, Trace, Finalize, JsData)]
struct UrlData {
    #[unsafe_ignore_trace]
    url: Url,
    /// The object `searchParams` returns, created with the URL so the
    /// attribute returns the same object each time.
    search_params: JsObject,
}

fn this_url(this: &JsValue) -> JsResult<Url> {
    this.as_object()
        .and_then(|object| {
            object
                .downcast_ref::<UrlData>()
                .map(|data| data.url.clone())
        })
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a URL")
                .into()
        })
}

/// The basic URL parser run on `input` against an optional `base`, or
/// `None` on failure (WHATWG URL Standard "API URL parser").
fn parse(input: &str, base: Option<&str>) -> Option<Url> {
    match base {
        Some(base) => Url::parse(base).ok()?.join(input).ok(),
        None => Url::parse(input).ok(),
    }
}

/// The URL and optional base of a constructor or `canParse` call.
fn parse_args(args: &[JsValue], context: &mut Context) -> JsResult<Option<Url>> {
    let input = args
        .first()
        .ok_or_else(|| JsNativeError::typ().with_message("url is required"))?
        .to_string(context)?
        .to_std_string_escaped();
    let base = match args.get(1) {
        Some(value) if !value.is_undefined() => {
            Some(value.to_string(context)?.to_std_string_escaped())
        }
        _ => None,
    };
    Ok(parse(&input, base.as_deref()))
}

/// The `URL` constructor; `params_prototype` is `URLSearchParams.prototype`,
/// so a script that replaces the global still gets real search params.
fn url_constructor(
    new_target: &JsValue,
    args: &[JsValue],
    params_prototype: &JsObject,
    context: &mut Context,
) -> JsResult<JsValue> {
    let new_target = new_target
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("URL constructor requires 'new'"))?;
    // cov:ignore: `URL.prototype` is a non-writable object, so this never fails.
    let prototype = new_target
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("constructor prototype is not an object")
        })?;
    let url = parse_args(args, context)?
        .ok_or_else(|| JsNativeError::typ().with_message("Invalid URL"))?;
    let search_params = super::url_search_params::params_object(
        params_prototype.clone(),
        url.query().unwrap_or_default(),
    );
    let object = JsObject::from_proto_and_data(Some(prototype), UrlData { url, search_params });
    Ok(object.into())
}

/// `URL.canParse(url, base)`.
fn url_can_parse(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(parse_args(args, context)?.is_some()))
}

fn string(value: &str) -> JsResult<JsValue> {
    Ok(JsValue::from(JsString::from(value)))
}

fn href(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    string(this_url(this)?.as_str())
}

fn origin(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    string(&this_url(this)?.origin().ascii_serialization())
}

fn protocol(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    string(&format!("{}:", this_url(this)?.scheme()))
}

fn username(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    string(this_url(this)?.username())
}

fn password(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    string(this_url(this)?.password().unwrap_or_default())
}

fn host(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let url = this_url(this)?;
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => string(&format!("{host}:{port}")),
        None => string(host),
    }
}

fn hostname(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    string(this_url(this)?.host_str().unwrap_or_default())
}

fn port(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    string(
        &this_url(this)?
            .port()
            .map_or_else(String::new, |port| port.to_string()),
    )
}

fn pathname(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    string(this_url(this)?.path())
}

/// `search` and `hash`: an empty component reads as `""`, not as the bare
/// delimiter.
fn delimited(delimiter: char, component: Option<&str>) -> JsResult<JsValue> {
    match component {
        Some(value) if !value.is_empty() => string(&format!("{delimiter}{value}")),
        _ => string(""),
    }
}

fn search(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    delimited('?', this_url(this)?.query())
}

fn hash(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    delimited('#', this_url(this)?.fragment())
}

fn search_params(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    this_url(this)?;
    Ok(this
        .as_object()
        .and_then(|object| {
            object
                .downcast_ref::<UrlData>()
                .map(|data| data.search_params.clone())
        })
        .map_or_else(JsValue::undefined, JsValue::from))
}

const ACCESSORS: &[(&str, boa_engine::native_function::NativeFunctionPointer)] = &[
    ("href", href),
    ("origin", origin),
    ("protocol", protocol),
    ("username", username),
    ("password", password),
    ("host", host),
    ("hostname", hostname),
    ("port", port),
    ("pathname", pathname),
    ("search", search),
    ("searchParams", search_params),
    ("hash", hash),
];

pub(crate) fn install(context: &mut Context, params_prototype: JsObject) -> JsResult<()> {
    let accessors: Vec<_> = ACCESSORS
        .iter()
        .map(|&(name, getter)| Ok((name, function(context, &format!("get {name}"), 0, getter)?)))
        .collect::<JsResult<_>>()?;
    let to_string = function(context, "toString", 0, href)?;
    let to_json = function(context, "toJSON", 0, href)?;
    let can_parse = function(context, "canParse", 1, url_can_parse)?;
    let constructor =
        NativeFunction::from_copy_closure_with_captures(url_constructor, params_prototype);
    let mut builder = ConstructorBuilder::new(context, constructor);
    builder.name("URL").length(1).constructor(true);
    let operation = Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE;
    for (name, getter) in accessors {
        builder.accessor(
            JsString::from(name),
            Some(getter),
            None,
            Attribute::ENUMERABLE | Attribute::CONFIGURABLE,
        );
    }
    builder.property(js_string!("toString"), to_string, operation);
    builder.property(js_string!("toJSON"), to_json, operation);
    builder.static_property(js_string!("canParse"), can_parse, operation);
    let standard = builder.build();
    let prototype = standard.prototype();
    let constructor = standard.constructor();
    set_to_string_tag(&prototype, "URL", context)?;
    context.register_global_property(
        js_string!("URL"),
        constructor,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;
    Ok(())
}
