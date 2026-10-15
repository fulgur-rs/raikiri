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

use std::sync::Arc;

use boa_engine::object::JsObject;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, Trace};

use super::interfaces::{Members, protos, wrap};
use super::node::mark_dirty;
use super::webidl::{
    dom_string, host_failure_with_message, this_element, throw_dom_exception, with_state,
};
use raikiri_traits::DecodedImage;

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
            // Only sRGB colors serialize as `#rrggbb` / `rgba()`; a color in
            // another space (`lab()`, `oklch()`, `color()`, a mix, or
            // relative color syntax) keeps that space when read back, which
            // 8-bit RGBA cannot represent, so such values are rejected.
            if !is_srgb_color_form(input) {
                return None;
            }
            Some(FillColor::Rgba([c.r, c.g, c.b, c.a]))
        }
        PropertyValue::BorderTopColor(BorderColor::CurrentColor) => Some(FillColor::CurrentColor),
        _ => None,
    }
}

/// Whether `input` (already known to be one `<color>`) is written as a hex
/// color, a named or system color, or a non-relative `rgb()`, `rgba()`,
/// `hsl()`, `hsla()` or `hwb()` function with no missing (`none`)
/// component: the forms CSS Color serializes as sRGB with the
/// HTML-compatible serialization.
fn is_srgb_color_form(input: &str) -> bool {
    use cssparser::Token;

    let mut input = cssparser::ParserInput::new(input);
    let mut parser = cssparser::Parser::new(&mut input);
    // A `<color>` that is not a function is a hex, named, or system color.
    let Ok(Token::Function(name)) = parser.next().cloned() else {
        return true;
    };
    if !["rgb", "rgba", "hsl", "hsla", "hwb"]
        .iter()
        .any(|known| name.eq_ignore_ascii_case(known))
    {
        return false;
    }
    parser
        .parse_nested_block(|block| {
            // `from` starts relative color syntax; `none` is a missing
            // component, which only the `color()` serialization can keep.
            // Reading every token also consumes the block to its end, as a
            // nested block must be.
            let mut legacy = true;
            while let Ok(token) = block.next() {
                if let Token::Ident(word) = token
                    && (word.eq_ignore_ascii_case("from") || word.eq_ignore_ascii_case("none"))
                {
                    legacy = false;
                }
            }
            Ok::<_, cssparser::ParseError<'_, ()>>(legacy)
        })
        .unwrap_or(false)
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
    let (width, _) = with_state(context, |s| s.host.document().canvas_size(index))?
        .unwrap_or(raikiri_dom::document::DEFAULT_CANVAS_SIZE);
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
    let (_, height) = with_state(context, |s| s.host.document().canvas_size(index))?
        .unwrap_or(raikiri_dom::document::DEFAULT_CANVAS_SIZE);
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
        Some(FillColor::CurrentColor) => match canvas_current_color(context, canvas)? {
            Some(rgba) => rgba,
            None => return Ok(JsValue::undefined()),
        },
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

/// `currentcolor` for a canvas fill (HTML Standard §4.12.5, `fillStyle`:
/// `currentcolor` is resolved with the canvas element): the element's
/// computed `color` at assignment time, or opaque black when the element is
/// not connected and so has no computed style. `None` when the host does
/// not report a computed `color`; the assignment is then ignored rather
/// than painted with a color the element does not have.
fn canvas_current_color(context: &mut Context, canvas: usize) -> JsResult<Option<[u8; 4]>> {
    let connected = with_state(context, |s| {
        super::tree::node_is_connected(s.host.document(), canvas)
    })?;
    if !connected {
        return Ok(Some([0, 0, 0, 255]));
    }
    super::style::ensure_flushed(context)?;
    let computed =
        with_state(context, |s| s.host.computed_value(canvas, "color"))?.map_err(|error| {
            host_failure_with_message(context, error, "computed style lookup failed")
        })?;
    Ok(match computed.as_deref().and_then(parse_fill_color) {
        Some(FillColor::Rgba(rgba)) => Some(rgba),
        _ => None,
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

/// The pixels of a `drawImage` source: straight-alpha RGBA8 rows that
/// cover the source's `size`, which is in CSS px and may be fractional.
struct SourcePixels {
    size: (f64, f64),
    width: u32,
    height: u32,
    rgba: SourceBytes,
}

/// Where a [`SourcePixels`] keeps its bytes: shared with the host's image
/// cache, or copied out of a canvas bitmap (which drawing may overwrite).
enum SourceBytes {
    Image(Arc<DecodedImage>),
    Canvas(Vec<u8>),
}

impl SourcePixels {
    fn pixel(&self, col: u32, row: u32) -> [u8; 4] {
        let bytes = match &self.rgba {
            SourceBytes::Image(image) => image.rgba.as_slice(),
            SourceBytes::Canvas(rgba) => rgba.as_slice(),
        };
        let off = ((row * self.width + col) as usize) * 4;
        bytes
            .get(off..off + 4)
            .and_then(|pixel| pixel.try_into().ok())
            .unwrap_or([0; 4])
    }
}

/// The `CanvasImageSource` an argument names (HTML Standard §4.12.5.1.15):
/// an `<img>` or `<canvas>` element; any other value is a `TypeError`, as
/// WebIDL union conversion requires.
enum ImageSource {
    Image(usize),
    Canvas(usize),
}

fn image_source(value: &JsValue, context: &mut Context) -> JsResult<ImageSource> {
    let not_a_source = || {
        JsNativeError::typ()
            .with_message("the image argument is not an HTMLImageElement or HTMLCanvasElement")
    };
    let index = this_element(value, context).map_err(|_| not_a_source())?;
    let (image, canvas) = with_state(context, |s| {
        let document = s.host.document();
        (
            super::image::is_html_image(document, index),
            document.is_canvas_element(index),
        )
    })?;
    match (image, canvas) {
        (true, _) => Ok(ImageSource::Image(index)),
        (_, true) => Ok(ImageSource::Canvas(index)),
        _ => Err(not_a_source().into()),
    }
}

/// "Check the usability of the image argument" (HTML Standard §4.12.5.1.15),
/// then read its pixels: `Ok(None)` when there is nothing to draw (an image
/// not yet loaded, or one whose pixels the host does not keep), and an
/// `InvalidStateError` for a broken image or a zero-area canvas. An image is
/// sized with the destination's output bitmap size, `bitmap_size`, as its
/// default object size.
fn source_pixels(
    context: &mut Context,
    source: ImageSource,
    bitmap_size: (u32, u32),
) -> JsResult<Option<SourcePixels>> {
    match source {
        ImageSource::Image(index) => {
            let loaded = with_state(context, |s| s.loaded_images.get(&index).cloned())?;
            match loaded {
                None => Ok(None),
                Some(None) => Err(throw_dom_exception(
                    context,
                    "InvalidStateError",
                    "the image is broken",
                )),
                Some(Some(url)) => {
                    let default_size = (f64::from(bitmap_size.0), f64::from(bitmap_size.1));
                    let pixels = with_state(context, |s| s.host.image_pixels(&url, default_size))?;
                    Ok(pixels.map(|pixels| SourcePixels {
                        size: pixels.size,
                        width: pixels.image.width,
                        height: pixels.image.height,
                        rgba: SourceBytes::Image(pixels.image),
                    }))
                }
            }
        }
        ImageSource::Canvas(index) => {
            let (size, rgba) = with_state(context, |s| {
                let document = s.host.document();
                let rgba = document
                    .canvas_bitmap_ref(index)
                    .filter(|bitmap| !bitmap.rgba.is_empty())
                    .map(|bitmap| bitmap.rgba.clone());
                (document.canvas_size(index), rgba)
            })?;
            let (width, height) = size.unwrap_or(raikiri_dom::document::DEFAULT_CANVAS_SIZE);
            if width == 0 || height == 0 {
                return Err(throw_dom_exception(
                    context,
                    "InvalidStateError",
                    "the canvas has no pixels",
                ));
            }
            // A canvas without materialized pixels is transparent black,
            // which source-over leaves unchanged.
            Ok(rgba.map(|rgba| SourcePixels {
                size: (f64::from(width), f64::from(height)),
                width,
                height,
                rgba: SourceBytes::Canvas(rgba),
            }))
        }
    }
}

/// A rectangle with negative extents flipped to positive ones, as
/// `drawImage` normalizes its source and destination rectangles.
fn normalized(x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    (x, y, w, h)
}

/// `ctx.drawImage(image, dx, dy)`, `drawImage(image, dx, dy, dw, dh)` and
/// `drawImage(image, sx, sy, sw, sh, dx, dy, dw, dh)` (HTML Standard
/// §4.12.5.1.15): paint the source rectangle of `image` into the
/// destination rectangle with source-over.
///
/// Each destination pixel whose center lies in the destination rectangle
/// takes the nearest source pixel; image smoothing is not modelled, so
/// scaled draws are not interpolated. Source pixels outside the image draw
/// nothing, which clips the destination in the same proportion as the
/// source rectangle is clipped to the image.
fn draw_image(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let canvas = with_canvas_context(this, context)?;
    // WebIDL overload resolution: the overloads take 3, 5 or 9 arguments,
    // and extra arguments beyond 9 are ignored.
    let count = args.len().min(9);
    if !matches!(count, 3 | 5 | 9) {
        return Err(JsNativeError::typ()
            .with_message(format!(
                "drawImage takes 3, 5 or 9 arguments, but {} were given",
                args.len()
            ))
            .into());
    }
    let source = image_source(&args[0], context)?;
    let mut numbers = Vec::with_capacity(count - 1);
    for i in 1..count {
        numbers.push(arg_double(args, i, context)?);
    }
    if numbers.iter().any(|number| !number.is_finite()) {
        return Ok(JsValue::undefined());
    }
    let (canvas_w, canvas_h) = with_state(context, |s| s.host.document().canvas_size(canvas))?
        .unwrap_or(raikiri_dom::document::DEFAULT_CANVAS_SIZE);
    let Some(pixels) = source_pixels(context, source, (canvas_w, canvas_h))? else {
        return Ok(JsValue::undefined());
    };
    let (image_w, image_h) = pixels.size;
    // The 3- and 5-argument forms draw the whole image, and the 3-argument
    // form draws it at its natural size.
    let full = [0.0, 0.0, image_w, image_h];
    let n = &numbers;
    let (source_rect, destination_rect) = match count {
        9 => ([n[0], n[1], n[2], n[3]], [n[4], n[5], n[6], n[7]]),
        5 => (full, [n[0], n[1], n[2], n[3]]),
        _ => (full, [n[0], n[1], image_w, image_h]),
    };
    let [sx, sy, sw, sh] = source_rect;
    let [dx, dy, dw, dh] = destination_rect;
    let (sx, sy, sw, sh) = normalized(sx, sy, sw, sh);
    let (dx, dy, dw, dh) = normalized(dx, dy, dw, dh);
    if sw == 0.0 || sh == 0.0 || dw == 0.0 || dh == 0.0 {
        return Ok(JsValue::undefined());
    }
    // The destination pixels the rectangle touches, clipped to the bitmap
    // so every coordinate fits an `i32`.
    let left = dx.floor().max(0.0);
    let top = dy.floor().max(0.0);
    let right = (dx + dw).ceil().min(f64::from(canvas_w));
    let bottom = (dy + dh).ceil().min(f64::from(canvas_h));
    if left >= right || top >= bottom {
        return Ok(JsValue::undefined());
    }
    let sample = |offset: f64, d: f64, dl: f64, s: f64, sl: f64, extent: f64, limit: u32| {
        // The pixel center, mapped from the destination to the source, then
        // from the source's CSS px to its raster pixels.
        let center = offset + 0.5;
        if center < d || center >= d + dl {
            return None;
        }
        let source = s + (center - d) * sl / dl;
        if source < 0.0 || source >= extent {
            return None;
        }
        let raster = (source * f64::from(limit) / extent).floor() as u32;
        Some(raster.min(limit.saturating_sub(1)))
    };
    with_state(context, |s| {
        s.host.document_mut().canvas_composite_rect(
            canvas,
            left as i32,
            top as i32,
            (right - left) as i32,
            (bottom - top) as i32,
            |col, row| {
                let source_col = sample(f64::from(col), dx, dw, sx, sw, image_w, pixels.width)?;
                let source_row = sample(f64::from(row), dy, dh, sy, sh, image_h, pixels.height)?;
                Some(pixels.pixel(source_col, source_row))
            },
        );
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
    methods: &[
        ("fillRect", 4, fill_rect),
        ("clearRect", 4, clear_rect),
        ("drawImage", 3, draw_image),
    ],
};

#[cfg(test)]
mod tests;
