//! `window.location`, `window.navigator`, `window.console`, and the
//! frame-identity members `parent`/`top`/`frames`/`opener` (`self`/`window`
//! are installed by [`super::interfaces::install`] already, onto the same
//! global object).
//!
//! `Location`/`Navigator` are real interfaces (illegal constructor,
//! `@@toStringTag` on the prototype) -- [`super::interfaces::install`]
//! builds them the same way as every other interface and passes their
//! prototypes into [`install`]. `console` has no interface object: the
//! Console Standard defines it as a WebIDL *namespace*, not an interface.
//! Exactly one instance of each singleton exists, reachable only through
//! `window`.

use boa_engine::native_function::NativeFunctionPointer;
use boa_engine::object::{JsObject, ObjectInitializer};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{Context, JsResult, JsValue, NativeFunction, js_string};

use super::interfaces::{Members, closure_function, function};
use super::node::js_str;
use super::webidl::with_state;

// ---- location --------------------------------------------------------

/// `window.location`'s components. Either derived from [`super::host::DocumentHost::document_url`],
/// or this fixed fallback when there is none (`about:blank`'s own component
/// breakdown: no host or port, opaque origin).
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
        // `Location.search`/`.hash` (WHATWG URL Standard): a delimiter with
        // nothing after it (`".../a?"`, `".../a#"`) reads back as `""`, not
        // the bare delimiter -- normalized once here, before either branch
        // below reads `search`/`hash`.
        let hash = normalize_bare_delimiter(hash);
        let search = normalize_bare_delimiter(search);
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
        let (host, pathname) = match rest.find('/') {
            Some(path_start) => (rest[..path_start].to_owned(), rest[path_start..].to_owned()),
            // No path segment at all (`"https://example.test"`): the
            // WHATWG URL Standard's own serialization still writes one
            // slash for a special scheme's empty path.
            None => (rest.to_owned(), "/".to_owned()),
        };
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

/// [`split_at_delimiter`]'s result, further collapsing a bare delimiter with
/// nothing after it (e.g. `"?"`, from a URL like `".../a?"`) to `""`.
fn normalize_bare_delimiter(component: String) -> String {
    if component.len() == 1 {
        String::new()
    } else {
        component
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

/// `Location`'s accessors are `[LegacyUnforgeable]`: own properties on each
/// instance (there is only ever one), not members of `Location.prototype`
/// -- `proto` (built by `interfaces::install`) carries only `@@toStringTag`
/// and backs `instanceof`/`Object.prototype.toString`.
fn location_object(context: &mut Context, proto: &JsObject) -> JsResult<JsObject> {
    let accessors: Vec<_> = LOCATION_ACCESSORS
        .iter()
        .map(|&(n, f)| {
            let getter = function(context, &format!("get {n}"), 0, f)?;
            let setter = function(context, &format!("set {n}"), 1, location_no_op)?;
            Ok((n, getter, setter))
        })
        .collect::<JsResult<_>>()?;
    let origin_getter = function(context, "get origin", 0, location_origin)?;
    let methods: Vec<_> = LOCATION_METHODS
        .iter()
        .map(|&(n, length, f)| Ok((n, function(context, n, length, f)?)))
        .collect::<JsResult<_>>()?;

    let attribute_attr = Attribute::ENUMERABLE | Attribute::CONFIGURABLE;
    let method_attr = Attribute::WRITABLE | Attribute::CONFIGURABLE;
    let mut builder = ObjectInitializer::with_native_data_and_proto((), proto.clone(), context);
    for (name, getter, setter) in accessors {
        builder.accessor(js_string!(name), Some(getter), Some(setter), attribute_attr);
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
    Ok(builder.build())
}

// ---- navigator ---------------------------------------------------------

/// A fixed `navigator.userAgent`: this runtime has no real user agent
/// string of its own to report, and content sniffing on it is out of scope.
const USER_AGENT: &str = "Mozilla/5.0 (compatible; raikiri)";

fn navigator_user_agent(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(js_str(USER_AGENT))
}

/// `Navigator.prototype`'s own members (unlike `Location`'s, a read-only
/// attribute with no per-instance state belongs on the prototype as usual).
pub(crate) const NAVIGATOR_MEMBERS: Members = Members {
    getters: &[("userAgent", navigator_user_agent)],
    accessors: &[],
    methods: &[],
};

/// The `Navigator` singleton has no own properties at all: `userAgent` is
/// inherited from `proto` (`Navigator.prototype`).
fn navigator_object(context: &mut Context, proto: &JsObject) -> JsResult<JsObject> {
    Ok(ObjectInitializer::with_native_data_and_proto((), proto.clone(), context).build())
}

// ---- console -------------------------------------------------------------

/// The `console.*` method a recorded message came from (Console Standard
/// `Console` namespace, "logging" section).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsoleLevel {
    Log,
    Error,
    Warn,
    Info,
    Debug,
}

/// One `console.*` call, recorded in call order. `State` is `pub(crate)`
/// with no accessor of its own, so nothing outside this crate can read
/// `State.console` back yet; only this runtime's own tests do, directly via
/// `with_state`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "level/message are only read back by this runtime's own tests so far, via with_state"
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

/// Build a `NativeFunction` that ignores its call arguments and always
/// returns a clone of `captures` -- used for `location`'s getter, which
/// must keep returning the exact same singleton object every time (`window
/// .location === window.location`), not rebuild one per call. `T: Trace`
/// (here, `JsObject`) so the capture is itself garbage-collector-visible;
/// see [`NativeFunction::from_copy_closure_with_captures`].
fn constant_getter(captures: JsObject) -> NativeFunction {
    NativeFunction::from_copy_closure_with_captures(
        |_this: &JsValue, _args: &[JsValue], captures: &JsObject, _context: &mut Context| {
            Ok(JsValue::from(captures.clone()))
        },
        captures,
    )
}

/// Define `window.location`/`navigator`/`console`, and the frame-identity
/// members `parent`/`top`/`frames`/`opener`. `location_proto`/
/// `navigator_proto` are `Location`/`Navigator`'s prototypes, built by
/// `interfaces::install` the same way as every other interface.
pub(crate) fn install(
    context: &mut Context,
    location_proto: &JsObject,
    navigator_proto: &JsObject,
) -> JsResult<()> {
    let location = location_object(context, location_proto)?;
    let navigator = navigator_object(context, navigator_proto)?;
    let console = console_object(context)?;

    // `location`: `[PutForwards=href, LegacyUnforgeable]` -- a real
    // accessor property, not a plain data property, so that writing to it
    // (even in strict mode) forwards to `href`'s own setter (a no-op here)
    // instead of throwing on a non-writable property. The getter always
    // returns the one singleton built above.
    let location_getter = closure_function(context, "get location", 0, constant_getter(location))?;
    let location_setter = function(context, "set location", 1, location_no_op)?;
    let location_descriptor = PropertyDescriptor::builder()
        .get(location_getter)
        .set(location_setter)
        .enumerable(true)
        .configurable(true)
        .build();
    context.global_object().define_property_or_throw(
        js_string!("location"),
        location_descriptor,
        context,
    )?;

    // `navigator`/`console`/`parent`/`frames`: `[Replaceable]` in the real
    // spec (reading returns the live value; writing defines an ordinary,
    // shadowing own property) -- approximated here as a plain writable data
    // property, which gives the same observable read/write behavior for a
    // simple assignment.
    let replaceable_attr = Attribute::WRITABLE | Attribute::CONFIGURABLE;
    context.register_global_property(js_string!("navigator"), navigator, replaceable_attr)?;
    context.register_global_property(js_string!("console"), console, replaceable_attr)?;
    let global = context.global_object();
    context.register_global_property(js_string!("parent"), global.clone(), replaceable_attr)?;
    context.register_global_property(js_string!("frames"), global.clone(), replaceable_attr)?;
    // `top`: `[LegacyUnforgeable]`, *not* `[Replaceable]` -- stays
    // non-writable, unlike `parent`/`frames` above.
    context.register_global_property(js_string!("top"), global, Attribute::CONFIGURABLE)?;
    // `opener`: a plain writable `any` attribute (not `[Replaceable]`,
    // just an ordinary read/write property), default `null`.
    context.register_global_property(js_string!("opener"), JsValue::null(), replaceable_attr)?;
    Ok(())
}

#[cfg(test)]
mod tests;
