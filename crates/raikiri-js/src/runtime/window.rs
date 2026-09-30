//! `window.location`, `window.navigator`, `window.console`, viewport
//! scrolling (`scrollX`/`scrollY`, `scroll`/`scrollTo`/`scrollBy`), and the
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
//!
//! Viewport scrolling (CSSOM View window scroll) keeps the window's own
//! scroll offset in shared state (see `super::State`'s viewport fields).
//! Print rendering has no viewport scrolling and ignores that offset: each
//! print page paints from its own content origin, so a scrolled live
//! document and its unserialized print rendering stay coherent by both
//! ignoring scroll equally (the live viewport scrolls, print does not).

pub(super) mod named;

use boa_engine::native_function::NativeFunctionPointer;
use boa_engine::object::{JsObject, ObjectInitializer};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{Context, JsNativeError, JsResult, JsString, JsValue, NativeFunction, js_string};

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

impl ConsoleLevel {
    /// The method name this level came from, lowercase -- what
    /// [`super::scripts::RunReport::console`] reports each entry's level as.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

/// One `console.*` call, recorded in call order. `State` is `pub(crate)`
/// with no accessor of its own, so nothing outside this crate can read
/// `State.console` back directly; [`super::scripts::RunReport::console`] is
/// the copy other callers see, and this runtime's own tests still read the
/// field directly via `with_state`.
#[derive(Debug, Clone, PartialEq, Eq)]
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

// ---- viewport scrolling --------------------------------------------------

// Viewport scroll offset (CSSOM View window scroll) lives in shared state,
// not in layout: the live host lays out a single screen page at the
// configured viewport size, and this offset is the window's scroll position
// over it. Print rendering ignores it (each print page paints from its own
// content origin), which keeps test and reference coherent: both scroll the
// live viewport the same way and both ignore scroll in print equally.

/// Clamp an absolute viewport offset: non-finite becomes zero, negatives
/// become zero, everything else is kept as-is. Positive overflow beyond the
/// scrollable size is kept here; max clamping needs layout and happens in
/// the host where that size is known, not in this shared-state store.
fn clamp_viewport_offset(value: f64) -> f64 {
    if !value.is_finite() {
        0.0
    } else {
        value.max(0.0)
    }
}

/// `window.scrollX` / `window.pageXOffset` (CSSOM View): the horizontal
/// viewport offset.
fn get_scroll_x(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let x = with_state(context, |s| s.viewport_scroll_x)?;
    Ok(JsValue::from(x))
}

/// `window.scrollY` / `window.pageYOffset` (CSSOM View): the vertical
/// viewport offset.
fn get_scroll_y(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let y = with_state(context, |s| s.viewport_scroll_y)?;
    Ok(JsValue::from(y))
}

/// A `ScrollBehavior` member: missing or `undefined` means instant; any other
/// value must be one of the three enum strings, otherwise this throws
/// `TypeError` instead of silently accepting an unknown behavior. Every
/// accepted behavior scrolls instantly: this runtime has no smooth-scroll
/// animation clock, so `smooth` does not interpolate.
fn scroll_behavior(options: &JsObject, context: &mut Context) -> JsResult<()> {
    let value = options.get(JsString::from("behavior"), context)?;
    if value.is_undefined() {
        return Ok(());
    }
    let behavior = value.to_string(context)?.to_std_string_escaped();
    if matches!(behavior.as_str(), "auto" | "instant" | "smooth") {
        Ok(())
    } else {
        Err(JsNativeError::typ()
            .with_message("behavior is not a valid ScrollBehavior")
            .into())
    }
}

/// An optional `unrestricted double` dictionary member: missing or
/// `undefined` means `default`; anything else runs `ToNumber` (which can
/// produce `NaN`, exactly as `unrestricted` allows) and the caller clamps.
fn dict_double(
    options: &JsObject,
    name: &str,
    default: Option<f64>,
    context: &mut Context,
) -> JsResult<Option<f64>> {
    let value = options.get(JsString::from(name), context)?;
    if value.is_undefined() {
        return Ok(default);
    }
    Ok(Some(value.to_number(context)?))
}

/// Whether `value` looks like a `ScrollToOptions` dictionary: a non-null
/// object. `null`/`undefined`/primitives take the numeric-argument path
/// (`Number(null)` is `0`, matching the numeric overload).
fn is_scroll_options(value: &JsValue) -> bool {
    value.as_object().is_some()
}

/// A numeric scroll argument: missing or `undefined` takes `default`
/// directly; anything else runs `ToNumber`, which can produce `NaN`.
fn num_arg(args: &[JsValue], i: usize, default: f64, context: &mut Context) -> JsResult<f64> {
    match args.get(i) {
        None => Ok(default),
        Some(v) if v.is_undefined() => Ok(default),
        Some(v) => v.to_number(context),
    }
}

/// `window.scrollTo` / `window.scroll` (CSSOM View): absolute viewport
/// scroll. Two numbers mean `(x, y)` with missing values defaulting to zero;
/// a single dictionary means `ScrollToOptions` (`left`/`top` default to the
/// current offset, so `scrollTo({top: 100})` keeps `x`). `behavior` is
/// validated but always scrolls instantly (see `scroll_behavior`).
fn scroll_to(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(options) = args
        .first()
        .filter(|v| is_scroll_options(v))
        .and_then(|v| v.as_object())
    {
        scroll_behavior(&options, context)?;
        let (current_x, current_y) =
            with_state(context, |s| (s.viewport_scroll_x, s.viewport_scroll_y))?;
        let x = match dict_double(&options, "left", None, context)? {
            Some(v) => clamp_viewport_offset(v),
            None => current_x,
        };
        let y = match dict_double(&options, "top", None, context)? {
            Some(v) => clamp_viewport_offset(v),
            None => current_y,
        };
        // `x`/`y` aliases (`scrollTo({x: 1, y: 2})`) are not part of
        // `ScrollToOptions`: only `left`/`top` are read, so they stay
        // ignored here rather than becoming a second way to scroll.
        with_state(context, |s| {
            s.viewport_scroll_x = x;
            s.viewport_scroll_y = y;
        })?;
        return Ok(JsValue::undefined());
    }
    let x = clamp_viewport_offset(num_arg(args, 0, 0.0, context)?);
    let y = clamp_viewport_offset(num_arg(args, 1, 0.0, context)?);
    with_state(context, |s| {
        s.viewport_scroll_x = x;
        s.viewport_scroll_y = y;
    })?;
    Ok(JsValue::undefined())
}

/// `window.scrollBy` (CSSOM View): relative viewport scroll. Two numbers are
/// `(dx, dy)` deltas defaulting to zero; a dictionary holds `left`/`top`
/// deltas the same way. A non-finite delta means no movement on that axis
/// (adding it would poison the offset to `NaN`), unlike absolute scroll
/// where non-finite means zero.
fn scroll_by(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(options) = args
        .first()
        .filter(|v| is_scroll_options(v))
        .and_then(|v| v.as_object())
    {
        scroll_behavior(&options, context)?;
        let dx = dict_double(&options, "left", Some(0.0), context)?.unwrap_or(0.0);
        let dy = dict_double(&options, "top", Some(0.0), context)?.unwrap_or(0.0);
        let dx = if dx.is_finite() { dx } else { 0.0 };
        let dy = if dy.is_finite() { dy } else { 0.0 };
        with_state(context, |s| {
            s.viewport_scroll_x = clamp_viewport_offset(s.viewport_scroll_x + dx);
            s.viewport_scroll_y = clamp_viewport_offset(s.viewport_scroll_y + dy);
        })?;
        return Ok(JsValue::undefined());
    }
    let dx = num_arg(args, 0, 0.0, context)?;
    let dy = num_arg(args, 1, 0.0, context)?;
    let dx = if dx.is_finite() { dx } else { 0.0 };
    let dy = if dy.is_finite() { dy } else { 0.0 };
    with_state(context, |s| {
        s.viewport_scroll_x = clamp_viewport_offset(s.viewport_scroll_x + dx);
        s.viewport_scroll_y = clamp_viewport_offset(s.viewport_scroll_y + dy);
    })?;
    Ok(JsValue::undefined())
}

const VIEWPORT_SCROLL_GETTERS: &[(&str, NativeFunctionPointer)] = &[
    ("scrollX", get_scroll_x),
    ("scrollY", get_scroll_y),
    ("pageXOffset", get_scroll_x),
    ("pageYOffset", get_scroll_y),
];

const VIEWPORT_SCROLL_METHODS: &[(&str, usize, NativeFunctionPointer)] = &[
    ("scroll", 0, scroll_to),
    ("scrollTo", 0, scroll_to),
    ("scrollBy", 0, scroll_by),
];

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

/// Define `window.location`/`navigator`/`console`, viewport scrolling, and
/// the frame-identity members `parent`/`top`/`frames`/`opener`.
/// `location_proto`/`navigator_proto` are `Location`/`Navigator`'s
/// prototypes, built by `interfaces::install` the same way as every other
/// interface.
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
    // A `?` on a call rustfmt wraps across lines leaves the never-taken
    // error-branch region on the closing line, so that line always reports
    // zero hits; binding the call first keeps the `?` on a one-line
    // statement instead (see `interfaces.rs`'s own `_result` bindings).
    let location_property_result = context.global_object().define_property_or_throw(
        js_string!("location"),
        location_descriptor,
        context,
    );
    location_property_result?;

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
    // Viewport scrolling (CSSOM View): `scrollX`/`scrollY` and their
    // `pageXOffset`/`pageYOffset` aliases are read-only attributes backed by
    // shared viewport state; `scroll`/`scrollTo`/`scrollBy` are operations.
    // Attributes are enumerable and configurable with only a getter, so a
    // non-strict write is ignored and a strict write throws, matching a
    // read-only WebIDL attribute. Operations are writable, enumerable, and
    // configurable, the same as the global timer operations.
    for (name, getter) in VIEWPORT_SCROLL_GETTERS {
        let get = function(context, &format!("get {name}"), 0, *getter)?;
        let descriptor = PropertyDescriptor::builder()
            .get(get)
            .enumerable(true)
            .configurable(true)
            .build();
        let scroll_attr_result = context.global_object().define_property_or_throw(
            JsString::from(*name),
            descriptor,
            context,
        );
        scroll_attr_result?;
    }
    for (name, length, func) in VIEWPORT_SCROLL_METHODS {
        let operation = function(context, name, *length, *func)?;
        let descriptor = PropertyDescriptor::builder()
            .value(operation)
            .writable(true)
            .enumerable(true)
            .configurable(true)
            .build();
        let scroll_op_result = context.global_object().define_property_or_throw(
            JsString::from(*name),
            descriptor,
            context,
        );
        scroll_op_result?;
    }
    named::install(context)?;
    Ok(())
}

#[cfg(test)]
mod tests;
