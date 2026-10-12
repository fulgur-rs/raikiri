//! HTML `<canvas>` element and 2d rendering context (HTML Standard §4.12.5).
//!
//! Minimal faithful subset needed by WPT `paintCanvases()` helpers:
//! `canvas.width`/`height` (unsigned long, 300×150 defaults, setting clears
//! the bitmap) and `canvas.getContext('2d')` returning a
//! `CanvasRenderingContext2D` with `fillStyle`/`fillRect`/`clearRect`.
//!
//! Only the `2d` context is supported; any other mode returns `null` per the
//! spec's "return null when the context id is not supported" rule. `fillStyle`
//! accepts any CSS `<color>` the cascade's color parser accepts (including
//! `currentcolor`); an unparsable assignment is ignored, retaining the
//! previous value. Transforms, gradients, patterns, paths, text, and image
//! smoothing are out of scope: `fillRect`/`clearRect` paint axis-aligned
//! rectangles directly into the live [`raikiri_dom::Document`] bitmap, which
//! paint later composites with `object-fit`/`object-position`.

use boa_engine::object::JsObject;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, Trace};

use super::interfaces::{Members, protos, wrap};
use super::node::mark_dirty;
use super::webidl::{dom_string, host_failure_with_message, this_element, with_state};

/// Native data behind a `CanvasRenderingContext2D` instance: the arena index
/// of its canvas element. The bitmap itself lives in
/// [`raikiri_dom::Document`], never here, so a canvas resized through its
/// width/height attributes automatically resizes the bitmap this context
/// paints into.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct CanvasContextData {
    #[unsafe_ignore_trace]
    pub canvas: usize,
}

fn with_canvas_context(this: &JsValue, _context: &mut Context) -> JsResult<usize> {
    this.as_object()
        .and_then(|o| o.downcast_ref::<CanvasContextData>().map(|d| d.canvas))
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a CanvasRenderingContext2D")
                .into()
        })
}

/// Whether `id` is an HTML `<canvas>` element, for brand checks on
/// `HTMLCanvasElement` members.
fn this_canvas(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_element(this, context)?;
    let is_canvas = with_state(context, |s| s.host.document().is_canvas_element(index))?;
    if is_canvas {
        Ok(index)
    } else {
        Err(JsNativeError::typ()
            .with_message("'this' is not an HTMLCanvasElement")
            .into())
    }
}

/// A `fillStyle` value parsed as a CSS `<color>` (HTML Standard §4.12.5
/// "fillStyle"), with the same color parser the cascade uses.
enum FillColor {
    Rgba([u8; 4]),
    /// `currentcolor`, resolved against the canvas element when assigned.
    CurrentColor,
}

/// Parse a CSS `<color>` for `fillStyle`. The whole string must be one
/// color (comments and surrounding whitespace aside); anything else, a
/// `var()` reference included, is `None` (the assignment is ignored, per
/// spec).
fn parse_fill_color(input: &str) -> Option<FillColor> {
    use raikiri_style::property::{BorderColor, PropertyValue};

    // `border-top-color` takes a plain `<color>` and keeps `currentcolor`
    // apart from resolved colors.
    match super::style::parse_property_value("border-top-color", input)? {
        PropertyValue::BorderTopColor(BorderColor::Resolved(c)) => {
            // The parser resolves a `currentcolor` nested in a color
            // function (`color-mix()`, relative color syntax) to black, not
            // to the canvas element's color, so such values are rejected
            // rather than painted with the wrong color.
            if mentions_current_color(input) {
                return None;
            }
            Some(FillColor::Rgba([c.r, c.g, c.b, c.a]))
        }
        PropertyValue::BorderTopColor(BorderColor::CurrentColor) => Some(FillColor::CurrentColor),
        _ => None,
    }
}

/// Whether `input` has a `currentcolor` identifier anywhere, including
/// inside functions. Blocks nested deeper than the scan follows count as a
/// mention, so such values are rejected instead of overflowing the stack.
fn mentions_current_color(input: &str) -> bool {
    const MAX_NESTING_DEPTH: usize = 128;

    fn scan(parser: &mut cssparser::Parser<'_, '_>, depth: usize) -> bool {
        while let Ok(token) = parser.next() {
            match token {
                cssparser::Token::Ident(name) if name.eq_ignore_ascii_case("currentcolor") => {
                    return true;
                }
                cssparser::Token::Function(_)
                | cssparser::Token::ParenthesisBlock
                | cssparser::Token::SquareBracketBlock
                | cssparser::Token::CurlyBracketBlock => {
                    if depth >= MAX_NESTING_DEPTH {
                        return true;
                    }
                    let found = parser
                        .parse_nested_block(|block| {
                            let found = scan(block, depth + 1);
                            // A nested block must be consumed to its end.
                            while block.next().is_ok() {}
                            Ok::<_, cssparser::ParseError<'_, ()>>(found)
                        })
                        .unwrap_or(false);
                    if found {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    let mut input = cssparser::ParserInput::new(input);
    scan(&mut cssparser::Parser::new(&mut input), 0)
}

/// The canvas serialization of a resolved color (HTML Standard §4.12.5,
/// "serialization of a color" for canvas): `#rrggbb` when opaque,
/// `rgba(r, g, b, a)` otherwise.
fn serialize_canvas_color([r, g, b, a]: [u8; 4]) -> String {
    if a == 255 {
        return format!("#{r:02x}{g:02x}{b:02x}");
    }
    // Shortest decimal that maps back to the same 8-bit alpha.
    let alpha = (1..=3)
        .map(|digits| format!("{:.*}", digits, f64::from(a) / 255.0))
        .find(|text| {
            text.parse::<f64>()
                .is_ok_and(|value| (value * 255.0).round() as u8 == a)
        })
        .unwrap_or_else(|| format!("{:.3}", f64::from(a) / 255.0));
    let alpha = alpha.trim_end_matches('0').trim_end_matches('.');
    let alpha = if alpha.is_empty() { "0" } else { alpha };
    format!("rgba({r}, {g}, {b}, {alpha})")
}

/// `canvas.width` (HTML Standard §4.12.5): the width content attribute value,
/// or 300 when missing or unparsable.
fn canvas_width(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_canvas(this, context)?;
    let (width, _) =
        with_state(context, |s| s.host.document().canvas_size(index))?.unwrap_or((300, 150));
    Ok(JsValue::from(width))
}

/// `canvas.width = value`: WebIDL unsigned-long conversion, then reflect to
/// the content attribute (which resizes and clears the bitmap).
fn set_canvas_width(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_canvas(this, context)?;
    let value = args.first().cloned().unwrap_or_default().to_u32(context)?;
    let result = with_state(context, |s| {
        s.host
            .document_mut()
            .set_element_attribute(index, "width", value.to_string())
    })?;
    if let Err(message) = result {
        // cov:ignore: "width" is always a valid XML name, so set_element_attribute cannot fail here
        return Err(boa_engine::JsError::from(
            JsNativeError::typ().with_message(message),
        ));
    }
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

/// `canvas.height` (HTML Standard §4.12.5): the height content attribute
/// value, or 150 when missing or unparsable.
fn canvas_height(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_canvas(this, context)?;
    let (_, height) =
        with_state(context, |s| s.host.document().canvas_size(index))?.unwrap_or((300, 150));
    Ok(JsValue::from(height))
}

/// `canvas.height = value`: WebIDL unsigned-long conversion, then reflect to
/// the content attribute (which resizes and clears the bitmap).
fn set_canvas_height(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_canvas(this, context)?;
    let value = args.first().cloned().unwrap_or_default().to_u32(context)?;
    let result = with_state(context, |s| {
        s.host
            .document_mut()
            .set_element_attribute(index, "height", value.to_string())
    })?;
    if let Err(message) = result {
        // cov:ignore: "height" is always a valid XML name, so set_element_attribute cannot fail here
        return Err(boa_engine::JsError::from(
            JsNativeError::typ().with_message(message),
        ));
    }
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

/// `canvas.getContext(mode)` (HTML Standard §4.12.5).
/// Returns the cached 2d context for `'2d'`, or `null` for any other mode.
/// A missing required argument throws `TypeError` per WebIDL.
fn get_context(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_canvas(this, context)?;
    let Some(first) = args.first() else {
        return Err(JsNativeError::typ()
            .with_message("getContext requires 1 argument")
            .into());
    };
    if first.is_null() || first.is_undefined() {
        return Ok(JsValue::null());
    }
    let mode = dom_string(args, 0, context)?;
    if mode != "2d" {
        return Ok(JsValue::null());
    }
    if let Some(existing) = with_state(context, |s| s.canvas_contexts.get(&index).cloned())? {
        return Ok(existing.into());
    }
    // Ensure a bounded bitmap exists before creating a drawing context.
    let allocation = with_state(context, |s| {
        s.host.document_mut().try_ensure_canvas_bitmap(index)
    })?;
    if !matches!(allocation, Ok(Some(_))) {
        return Ok(JsValue::null());
    }
    let proto = protos(context).canvas_rendering_context_2d.clone();
    let object = JsObject::from_proto_and_data(Some(proto), CanvasContextData { canvas: index });
    with_state(context, |s| {
        s.canvas_contexts.insert(index, object.clone());
    })?;
    Ok(object.into())
}

/// `ctx.canvas`: the canvas element this context was created for.
fn context_canvas(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let canvas = with_canvas_context(this, context)?;
    wrap(context, canvas).map(JsValue::from)
}

/// `ctx.fillStyle` getter: the serialization of the last successfully
/// assigned color, or `"#000000"` per spec default.
fn fill_style(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let canvas = with_canvas_context(this, context)?;
    let style = with_state(context, |s| {
        s.canvas_fill_styles
            .get(&canvas)
            .map(|(serialized, _)| serialized.clone())
            .unwrap_or_else(|| "#000000".to_owned())
    })?;
    Ok(JsValue::from(JsString::from(style.as_str())))
}

/// `ctx.fillStyle = value`: parse as CSS `<color>`; on failure ignore the
/// assignment and retain the previous value (HTML Standard §4.12.5).
fn set_fill_style(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let canvas = with_canvas_context(this, context)?;
    let value = dom_string(args, 0, context)?;
    let rgba = match parse_fill_color(&value) {
        Some(FillColor::Rgba(rgba)) => rgba,
        Some(FillColor::CurrentColor) => canvas_current_color(context, canvas)?,
        None => return Ok(JsValue::undefined()),
    };
    // The getter returns the parsed color's serialization, not the text
    // that was assigned.
    with_state(context, |s| {
        s.canvas_fill_styles
            .insert(canvas, (serialize_canvas_color(rgba), rgba));
    })?;
    Ok(JsValue::undefined())
}

/// `currentcolor` for a canvas fill: the canvas element's computed `color`
/// at assignment time, or opaque black when there is none (HTML Standard
/// §4.12.5, `fillStyle`: `currentcolor` is resolved with the canvas
/// element).
fn canvas_current_color(context: &mut Context, canvas: usize) -> JsResult<[u8; 4]> {
    super::style::ensure_flushed(context)?;
    let computed =
        with_state(context, |s| s.host.computed_value(canvas, "color"))?.map_err(|error| {
            host_failure_with_message(context, error, "computed style lookup failed")
        })?;
    Ok(match computed.as_deref().and_then(parse_fill_color) {
        Some(FillColor::Rgba(rgba)) => rgba,
        _ => [0, 0, 0, 255],
    })
}

fn arg_double(args: &[JsValue], i: usize, context: &mut Context) -> JsResult<f64> {
    args.get(i).cloned().unwrap_or_default().to_number(context)
}

fn normalize_rect(x: f64, y: f64, w: f64, h: f64) -> Option<(i32, i32, i32, i32)> {
    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
        return None;
    }
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    if w == 0.0 || h == 0.0 {
        return None;
    }
    // Bitmap pixels are integer-addressed; fractional edges cover whole
    // pixels the same way the reference renderer snaps them for these
    // integer test fixtures.
    Some((
        x.floor() as i32,
        y.floor() as i32,
        w.ceil() as i32,
        h.ceil() as i32,
    ))
}

fn current_fill_rgba(context: &mut Context, canvas: usize) -> JsResult<[u8; 4]> {
    with_state(context, |s| {
        Ok(s.canvas_fill_styles
            .get(&canvas)
            .map(|(_, rgba)| *rgba)
            .unwrap_or([0, 0, 0, 255]))
    })?
}

/// `ctx.fillRect(x, y, w, h)`: paint the rectangle with the current
/// `fillStyle`. Non-finite arguments are a no-op per spec; zero-area is a
/// no-op; negative sizes normalize by flipping the origin.
fn fill_rect(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if args.len() < 4 {
        return Err(JsNativeError::typ()
            .with_message("fillRect requires 4 arguments")
            .into());
    }
    let canvas = with_canvas_context(this, context)?;
    let x = arg_double(args, 0, context)?;
    let y = arg_double(args, 1, context)?;
    let w = arg_double(args, 2, context)?;
    let h = arg_double(args, 3, context)?;
    let Some((x, y, w, h)) = normalize_rect(x, y, w, h) else {
        return Ok(JsValue::undefined());
    };
    let rgba = current_fill_rgba(context, canvas)?;
    with_state(context, |s| {
        s.host
            .document_mut()
            .canvas_fill_rect(canvas, x, y, w, h, rgba);
    })?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

/// `ctx.clearRect(x, y, w, h)`: clear to transparent black. Argument handling
/// mirrors [`fill_rect`].
fn clear_rect(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if args.len() < 4 {
        return Err(JsNativeError::typ()
            .with_message("clearRect requires 4 arguments")
            .into());
    }
    let canvas = with_canvas_context(this, context)?;
    let x = arg_double(args, 0, context)?;
    let y = arg_double(args, 1, context)?;
    let w = arg_double(args, 2, context)?;
    let h = arg_double(args, 3, context)?;
    let Some((x, y, w, h)) = normalize_rect(x, y, w, h) else {
        return Ok(JsValue::undefined());
    };
    with_state(context, |s| {
        s.host.document_mut().canvas_clear_rect(canvas, x, y, w, h);
    })?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

pub(crate) const HTML_CANVAS_ELEMENT_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[
        ("width", canvas_width, set_canvas_width),
        ("height", canvas_height, set_canvas_height),
    ],
    methods: &[("getContext", 1, get_context)],
};

pub(crate) const CANVAS_RENDERING_CONTEXT_2D_MEMBERS: Members = Members {
    getters: &[("canvas", context_canvas)],
    accessors: &[("fillStyle", fill_style, set_fill_style)],
    methods: &[("fillRect", 4, fill_rect), ("clearRect", 4, clear_rect)],
};

#[cfg(test)]
mod tests;
