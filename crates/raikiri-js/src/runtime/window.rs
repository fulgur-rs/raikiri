//! `window.location`, `window.navigator`, `window.console`, and the
//! frame-identity members `parent`/`top`/`frames`/`opener` (`self`/`window`
//! are installed by [`super::interfaces::install`] already, onto the same
//! global object).
//!
//! `location`, `navigator`, and `console` are exposed as plain singleton
//! objects, not as interfaces with their own constructor: there is exactly
//! one of each, reachable only through `window`, and nothing here ever
//! constructs a second one.

use boa_engine::native_function::NativeFunctionPointer;
use boa_engine::object::{JsObject, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, JsResult, JsValue, js_string};

use super::interfaces::function;
use super::node::js_str;
use super::webidl::with_state;

// ---- location --------------------------------------------------------

/// `window.location`'s components. Either derived from [`super::host::
/// DocumentHost::document_url`], or this fixed fallback when there is none
/// (`about:blank`'s own component breakdown: no host or port, opaque
/// origin).
struct LocationParts {
    href: String,
    protocol: String,
    host: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
    origin: String,
}

impl LocationParts {
    fn about_blank() -> Self {
        Self {
            href: "about:blank".to_owned(),
            protocol: "about:".to_owned(),
            host: String::new(),
            hostname: String::new(),
            port: String::new(),
            pathname: "blank".to_owned(),
            search: String::new(),
            hash: String::new(),
            origin: "null".to_owned(),
        }
    }

    /// Split `url` into `Location`'s components. `url` is already a valid
    /// absolute URL string (see [`super::host::DocumentHost::document_url`]'s
    /// own contract) -- this is component extraction over an already-valid
    /// string (WHATWG URL Standard's own serialization for each component),
    /// not a general URL parser: it does not validate, percent-decode, or
    /// handle a userinfo (`user:pass@`) or a bracketed IPv6 host literal,
    /// none of which this runtime's own callers ever produce. A string
    /// outside the `scheme://host[:port][/path][?query][#fragment]` shape
    /// still degrades without panicking, but its split is not meaningful.
    fn parse(url: &str) -> Self {
        let (before_hash, hash) = split_at_delimiter(url, '#');
        let (before_search, search) = split_at_delimiter(before_hash, '?');
        let Some(scheme_end) = before_search.find("://") else {
            return Self {
                href: url.to_owned(),
                protocol: String::new(),
                host: String::new(),
                hostname: String::new(),
                port: String::new(),
                pathname: before_search.to_owned(),
                search,
                hash,
                origin: "null".to_owned(),
            };
        };
        let protocol = format!("{}:", &before_search[..scheme_end]);
        let rest = &before_search[scheme_end + 3..];
        let path_start = rest.find('/').unwrap_or(rest.len());
        let host = rest[..path_start].to_owned();
        let pathname = rest[path_start..].to_owned();
        let (hostname, port) = match host.split_once(':') {
            Some((h, p)) => (h.to_owned(), p.to_owned()),
            None => (host.clone(), String::new()),
        };
        let origin = format!("{protocol}//{host}");
        Self {
            href: url.to_owned(),
            protocol,
            host,
            hostname,
            port,
            pathname,
            search,
            hash,
            origin,
        }
    }
}

/// Split `s` right before the first `delimiter`, keeping `delimiter` itself
/// in the second half (`"a?b"` split on `'?'` is `("a", "?b")`); `(s, "")`
/// when `delimiter` does not occur.
fn split_at_delimiter(s: &str, delimiter: char) -> (&str, String) {
    match s.find(delimiter) {
        Some(i) => (&s[..i], s[i..].to_owned()),
        None => (s, String::new()),
    }
}

fn location_parts(context: &mut Context) -> JsResult<LocationParts> {
    let url = with_state(context, |s| s.host.document_url())?;
    Ok(match url {
        Some(url) => LocationParts::parse(&url),
        None => LocationParts::about_blank(),
    })
}

fn location_href(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.href))
}
fn location_protocol(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.protocol))
}
fn location_host(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.host))
}
fn location_hostname(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.hostname))
}
fn location_port(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.port))
}
fn location_pathname(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.pathname))
}
fn location_search(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.search))
}
fn location_hash(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.hash))
}
fn location_origin(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(&location_parts(context)?.origin))
}

fn location_to_string(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    location_href(this, args, context)
}

/// Every writable `Location` accessor's setter, and the body of `assign`/
/// `replace`/`reload`: navigation is out of scope for this runtime, so all
/// six are no-ops.
fn location_no_op(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

const LOCATION_ACCESSORS: &[(&str, NativeFunctionPointer)] = &[
    ("href", location_href),
    ("protocol", location_protocol),
    ("host", location_host),
    ("hostname", location_hostname),
    ("port", location_port),
    ("pathname", location_pathname),
    ("search", location_search),
    ("hash", location_hash),
];

const LOCATION_METHODS: &[(&str, usize, NativeFunctionPointer)] = &[
    ("toString", 0, location_to_string),
    ("assign", 1, location_no_op),
    ("replace", 1, location_no_op),
    ("reload", 0, location_no_op),
];

fn location_object(context: &mut Context) -> JsResult<JsObject> {
    let no_op_setter = function(context, "set location", 1, location_no_op)?;
    let accessors: Vec<_> = LOCATION_ACCESSORS
        .iter()
        .map(|&(n, f)| Ok((n, function(context, &format!("get {n}"), 0, f)?)))
        .collect::<JsResult<_>>()?;
    let origin_getter = function(context, "get origin", 0, location_origin)?;
    let methods: Vec<_> = LOCATION_METHODS
        .iter()
        .map(|&(n, length, f)| Ok((n, function(context, n, length, f)?)))
        .collect::<JsResult<_>>()?;

    let attribute_attr = Attribute::ENUMERABLE | Attribute::CONFIGURABLE;
    let method_attr = Attribute::WRITABLE | Attribute::CONFIGURABLE;
    let mut builder = ObjectInitializer::new(context);
    for (name, getter) in accessors {
        builder.accessor(
            js_string!(name),
            Some(getter),
            Some(no_op_setter.clone()),
            attribute_attr,
        );
    }
    builder.accessor(
        js_string!("origin"),
        Some(origin_getter),
        None,
        attribute_attr,
    );
    for (name, method) in methods {
        builder.property(js_string!(name), method, method_attr);
    }
    let object = builder.build();
    super::interfaces::set_to_string_tag(&object, "Location", context)?;
    Ok(object)
}

// ---- navigator ---------------------------------------------------------

/// A fixed `navigator.userAgent`: this runtime has no real user agent
/// string of its own to report, and content sniffing on it is out of scope.
const USER_AGENT: &str = "Mozilla/5.0 (compatible; raikiri)";

fn navigator_object(context: &mut Context) -> JsResult<JsObject> {
    let attr = Attribute::ENUMERABLE | Attribute::CONFIGURABLE;
    let object = ObjectInitializer::new(context)
        .property(js_string!("userAgent"), js_str(USER_AGENT), attr)
        .build();
    super::interfaces::set_to_string_tag(&object, "Navigator", context)?;
    Ok(object)
}

// ---- console -------------------------------------------------------------

/// The `console.*` method a recorded message came from (Console Standard
/// `Console` namespace, "logging" section).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "every level is produced by the matching console method; nothing outside this \
              runtime's own tests reads the recorded level back yet"
)]
pub(crate) enum ConsoleLevel {
    Log,
    Error,
    Warn,
    Info,
    Debug,
}

/// One `console.*` call, recorded in call order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "recorded for the embedder to report; nothing outside this runtime's own tests \
              reads a recorded message back yet"
)]
pub(crate) struct ConsoleMessage {
    pub level: ConsoleLevel,
    pub message: String,
}

/// `ToString` every argument and join with a single space (this runtime has
/// no format-specifier support -- `console.log('%s', x)` prints literally).
fn console_message(args: &[JsValue], context: &mut Context) -> JsResult<String> {
    let mut parts = Vec::with_capacity(args.len());
    for arg in args {
        parts.push(arg.to_string(context)?.to_std_string_escaped());
    }
    Ok(parts.join(" "))
}

fn console_record(
    level: ConsoleLevel,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let message = console_message(args, context)?;
    with_state(context, |s| {
        s.console.push(ConsoleMessage { level, message });
    })?;
    Ok(JsValue::undefined())
}

fn console_log(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    console_record(ConsoleLevel::Log, args, context)
}
fn console_error(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    console_record(ConsoleLevel::Error, args, context)
}
fn console_warn(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    console_record(ConsoleLevel::Warn, args, context)
}
fn console_info(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    console_record(ConsoleLevel::Info, args, context)
}
fn console_debug(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    console_record(ConsoleLevel::Debug, args, context)
}

const CONSOLE_METHODS: &[(&str, NativeFunctionPointer)] = &[
    ("log", console_log),
    ("error", console_error),
    ("warn", console_warn),
    ("info", console_info),
    ("debug", console_debug),
];

fn console_object(context: &mut Context) -> JsResult<JsObject> {
    let methods: Vec<_> = CONSOLE_METHODS
        .iter()
        .map(|&(n, f)| Ok((n, function(context, n, 0, f)?)))
        .collect::<JsResult<_>>()?;
    let attr = Attribute::WRITABLE | Attribute::CONFIGURABLE;
    let mut builder = ObjectInitializer::new(context);
    for (name, method) in methods {
        builder.property(js_string!(name), method, attr);
    }
    let object = builder.build();
    // The Console Standard defines `console` as a WebIDL *namespace*, not an
    // interface -- its `Symbol.toStringTag` is the namespace identifier
    // itself, lowercase, unlike every interface installed through
    // `interfaces::interface` (which is why real browsers have no global
    // `Console` constructor either, just this one object).
    super::interfaces::set_to_string_tag(&object, "console", context)?;
    Ok(object)
}

// ---- install ---------------------------------------------------------

/// Define `window.location`/`navigator`/`console`, and the frame-identity
/// members `parent`/`top`/`frames`/`opener`.
pub(crate) fn install(context: &mut Context) -> JsResult<()> {
    let location = location_object(context)?;
    let navigator = navigator_object(context)?;
    let console = console_object(context)?;
    // Non-writable, like `window`/`self` (`interfaces::install`): assigning
    // to `window.location` must not silently replace it with an unrelated
    // value, since navigating through it is a no-op rather than an error.
    let singleton_attr = Attribute::CONFIGURABLE;
    context.register_global_property(js_string!("location"), location, singleton_attr)?;
    context.register_global_property(js_string!("navigator"), navigator, singleton_attr)?;
    context.register_global_property(js_string!("console"), console, singleton_attr)?;

    let global = context.global_object();
    context.register_global_property(js_string!("parent"), global.clone(), singleton_attr)?;
    context.register_global_property(js_string!("top"), global.clone(), singleton_attr)?;
    context.register_global_property(js_string!("frames"), global, singleton_attr)?;
    context.register_global_property(js_string!("opener"), JsValue::null(), singleton_attr)?;
    Ok(())
}

#[cfg(test)]
mod tests;
