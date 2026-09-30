//! HTML `<canvas>` element and 2d rendering context (HTML Standard §4.12.5).
//!
//! Minimal faithful subset needed by WPT `paintCanvases()` helpers:
//! `canvas.width`/`height` (unsigned long, 300×150 defaults, setting clears
//! the bitmap) and `canvas.getContext('2d')` returning a
//! `CanvasRenderingContext2D` with `fillStyle`/`fillRect`/`clearRect`.
//!
//! Only the `2d` context is supported; any other mode returns `null` per the
//! spec's "return null when the context id is not supported" rule. `fillStyle`
//! accepts CSS `<color>` strings (named, hex, `rgb()`/`rgba()`,
//! `transparent`); an unparsable assignment is ignored, retaining the
//! previous value. Transforms, gradients, patterns, paths, text, and image
//! smoothing are out of scope: `fillRect`/`clearRect` paint axis-aligned
//! rectangles directly into the live [`raikiri_dom::Document`] bitmap, which
//! paint later composites with `object-fit`/`object-position`.

use boa_engine::object::JsObject;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, Trace};

use super::interfaces::{Members, protos, wrap};
use super::node::mark_dirty;
use super::webidl::{dom_string, this_element, with_state};

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

/// Parse a CSS `<color>` for `fillStyle` (HTML Standard §4.12.5 "fillStyle").
/// Returns premultiplied-free RGBA. Supports named colors (via cssparser's
/// CSS Color 4 table, so `green` is `0,128,0`), `#rgb`/`#rrggbb`/`#rrggbbaa`,
/// `rgb()`/`rgba()` with numbers or percentages, `transparent`, and the
/// `grey`/`gray` aliases through the same table. Anything else is `None`
/// (the assignment is ignored, per spec).
fn parse_fill_color(input: &str) -> Option<[u8; 4]> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("transparent") {
        return Some([0, 0, 0, 0]);
    }
    if let Some(hex) = trimmed.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    // Named colors are ASCII case-insensitive per CSS.
    let lower = trimmed.to_ascii_lowercase();
    if let Ok((r, g, b)) = cssparser::color::parse_named_color(&lower) {
        return Some([r, g, b, 255]);
    }
    parse_rgb_function(trimmed)
}

fn parse_hex_color(hex: &str) -> Option<[u8; 4]> {
    let bytes = hex.as_bytes();
    let digit = |b: u8| (b as char).to_digit(16).map(|v| v as u8);
    match bytes.len() {
        3 => {
            let r = digit(bytes[0])?;
            let g = digit(bytes[1])?;
            let b = digit(bytes[2])?;
            Some([r * 17, g * 17, b * 17, 255])
        }
        4 => {
            let r = digit(bytes[0])?;
            let g = digit(bytes[1])?;
            let b = digit(bytes[2])?;
            let a = digit(bytes[3])?;
            Some([r * 17, g * 17, b * 17, a * 17])
        }
        6 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            Some([
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
                255,
            ])
        }
        8 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            Some([
                ((v >> 24) & 0xFF) as u8,
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
            ])
        }
        _ => None,
    }
}

fn parse_rgb_component(token: &str, is_alpha: bool) -> Option<f32> {
    let token = token.trim();
    if let Some(percent) = token.strip_suffix('%') {
        let v: f32 = percent.trim().parse().ok()?;
        if !v.is_finite() {
            return None;
        }
        if is_alpha {
            Some((v / 100.0).clamp(0.0, 1.0))
        } else {
            Some((v * 255.0 / 100.0).clamp(0.0, 255.0))
        }
    } else {
        let v: f32 = token.parse().ok()?;
        if !v.is_finite() {
            return None;
        }
        if is_alpha {
            Some(v.clamp(0.0, 1.0))
        } else {
            Some(v.clamp(0.0, 255.0))
        }
    }
}

fn parse_rgb_function(input: &str) -> Option<[u8; 4]> {
    let lower = input.to_ascii_lowercase();
    let (name, args) = if let Some(rest) = lower.strip_prefix("rgba(") {
        ("rgba", rest)
    } else if let Some(rest) = lower.strip_prefix("rgb(") {
        ("rgb", rest)
    } else {
        return None;
    };
    let args = args.strip_suffix(')')?;
    // Legacy `rgba(r, g, b, a)` with commas takes precedence when present:
    // four comma-separated parts parse directly, before any space/slash
    // normalization that would otherwise merge them into four whitespace
    // tokens and reject the shape.
    if name == "rgba" && args.contains(',') {
        let legacy: Vec<&str> = args.split(',').map(str::trim).collect();
        if legacy.len() != 4 {
            return None;
        }
        let r = parse_rgb_component(legacy[0], false)?;
        let g = parse_rgb_component(legacy[1], false)?;
        let b = parse_rgb_component(legacy[2], false)?;
        let a = parse_rgb_component(legacy[3], true)?;
        return Some([
            r.round() as u8,
            g.round() as u8,
            b.round() as u8,
            (a * 255.0).round() as u8,
        ]);
    }
    // Modern space/slash syntax (`rgb(255 0 0 / 50%)`) and legacy `rgb()`
    // with commas (three parts) both appear; normalize commas to spaces and
    // split a slash-separated alpha.
    let normalized = args.replace(',', " ");
    let (rgb_part, alpha_part) = match normalized.split_once('/') {
        Some((rgb, alpha)) => (rgb, Some(alpha)),
        None => (normalized.as_str(), None),
    };
    let rgb_tokens: Vec<&str> = rgb_part
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect();
    if rgb_tokens.len() != 3 {
        return None;
    }
    let r = parse_rgb_component(rgb_tokens[0], false)?;
    let g = parse_rgb_component(rgb_tokens[1], false)?;
    let b = parse_rgb_component(rgb_tokens[2], false)?;
    let a = match alpha_part {
        Some(alpha) => parse_rgb_component(alpha, true)?,
        None => 1.0,
    };
    Some([
        r.round() as u8,
        g.round() as u8,
        b.round() as u8,
        (a * 255.0).round() as u8,
    ])
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
    // Ensure a bitmap exists at the current size so paint has pixels even
    // before the first fillRect.
    with_state(context, |s| {
        s.host.document_mut().ensure_canvas_bitmap(index);
    })?;
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

/// `ctx.fillStyle` getter: the last successfully assigned serialization, or
/// `"#000000"` per spec default.
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
    if let Some(rgba) = parse_fill_color(&value) {
        with_state(context, |s| {
            s.canvas_fill_styles.insert(canvas, (value, rgba));
        })?;
    }
    Ok(JsValue::undefined())
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
