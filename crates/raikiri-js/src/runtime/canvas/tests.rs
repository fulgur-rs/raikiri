use super::super::test_host::StubHost;
use super::super::{DomRuntime, RuntimeError};

fn rt() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

#[test]
fn canvas_prototypes_form_the_dom_chain() {
    let mut rt = rt();
    ok(
        &mut rt,
        "Object.getPrototypeOf(HTMLCanvasElement.prototype) === HTMLElement.prototype",
    );
    ok(
        &mut rt,
        "Object.prototype.toString.call(document.createElement('canvas')) === '[object HTMLCanvasElement]'",
    );
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         Object.prototype.toString.call(ctx) === '[object CanvasRenderingContext2D]'",
    );
}

#[test]
fn canvas_constructors_are_illegal() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { new HTMLCanvasElement(); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { new CanvasRenderingContext2D(); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { HTMLCanvasElement(); false } catch (e) { e instanceof TypeError }",
    );
}

#[test]
fn canvas_width_and_height_default_and_reflect() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); c.width === 300 && c.height === 150",
    );
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); c.width = 50; c.height = 100; \
         c.width === 50 && c.height === 100 && \
         c.getAttribute('width') === '50' && c.getAttribute('height') === '100'",
    );
}

#[test]
fn canvas_width_setter_clears_the_bitmap() {
    let (mut host, _, _, body) = StubHost::page();
    let canvas = host.document.create_detached_element("canvas").unwrap();
    host.document.append_child(body, canvas).unwrap();
    host.document.mark_in_document_flags();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var c = document.getElementsByTagName('canvas')[0]; \
         c.width = 4; c.height = 4; \
         var ctx = c.getContext('2d'); ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 4, 4);",
    )
    .unwrap();
    rt.evaluate("var c = document.getElementsByTagName('canvas')[0]; c.width = 4;")
        .unwrap();
    let host = rt.into_host();
    let bitmap = host
        .document()
        .canvas_bitmap(canvas)
        .expect("resizing keeps a cleared bitmap");
    assert_eq!((bitmap.width, bitmap.height), (4, 4));
    assert!(
        bitmap.rgba.iter().all(|&b| b == 0),
        "setting width clears the bitmap to transparent black"
    );
}

#[test]
fn get_context_caches_and_rejects_other_modes() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); \
         c.getContext('2d') === c.getContext('2d') && \
         c.getContext('2d') instanceof CanvasRenderingContext2D",
    );
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); \
         c.getContext('webgl') === null && c.getContext('bitmaprenderer') === null",
    );
    let err = rt.evaluate("document.createElement('canvas').getContext()");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "missing mode throws, got {err:?}"
    );
}

#[test]
fn get_context_brand_checks_the_canvas() {
    let mut rt = rt();
    let err = rt.evaluate("HTMLCanvasElement.prototype.getContext.call(document.body, '2d')");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "non-canvas receiver throws, got {err:?}"
    );
}

#[test]
fn fill_style_parses_named_hex_and_rgb_and_ignores_invalid() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         ctx.fillStyle === '#000000'",
    );
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         ctx.fillStyle = 'blue'; ctx.fillStyle === 'blue'",
    );
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         ctx.fillStyle = 'blue'; ctx.fillStyle = 'not-a-color'; ctx.fillStyle === 'blue'",
    );
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         ctx.fillStyle = '#ff0000'; ctx.fillStyle === '#ff0000'",
    );
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         ctx.fillStyle = 'rgb(0, 128, 0)'; ctx.fillStyle === 'rgb(0, 128, 0)'",
    );
}

#[test]
fn fill_rect_paints_and_clear_rect_clears() {
    let (mut host, _, _, body) = StubHost::page();
    let canvas = host.document.create_detached_element("canvas").unwrap();
    host.document.append_child(body, canvas).unwrap();
    host.document.mark_in_document_flags();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var c = document.getElementsByTagName('canvas')[0]; \
         c.width = 4; c.height = 4; \
         var ctx = c.getContext('2d'); \
         ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 4, 4); \
         ctx.clearRect(0, 0, 2, 2);",
    )
    .unwrap();
    let host = rt.into_host();
    let bitmap = host
        .document()
        .canvas_bitmap(canvas)
        .expect("bitmap exists");
    assert_eq!((bitmap.width, bitmap.height), (4, 4));
    // Top-left 2x2 cleared to transparent, rest stays red.
    assert_eq!(&bitmap.rgba[0..4], &[0, 0, 0, 0]);
    assert_eq!(
        &bitmap.rgba[(3 * 4 + 3) * 4..(3 * 4 + 4) * 4],
        &[255, 0, 0, 255]
    );
}

#[test]
fn fill_rect_clips_and_ignores_non_finite() {
    let (mut host, _, _, body) = StubHost::page();
    let canvas = host.document.create_detached_element("canvas").unwrap();
    host.document.append_child(body, canvas).unwrap();
    host.document.mark_in_document_flags();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var c = document.getElementsByTagName('canvas')[0]; \
         c.width = 4; c.height = 4; \
         var ctx = c.getContext('2d'); \
         ctx.fillStyle = 'blue'; ctx.fillRect(-2, -2, 4, 4); \
         ctx.fillStyle = 'red'; ctx.fillRect(NaN, 0, 2, 2); ctx.fillRect(0, 0, Infinity, 2);",
    )
    .unwrap();
    let host = rt.into_host();
    let bitmap = host
        .document()
        .canvas_bitmap(canvas)
        .expect("bitmap exists");
    // Only the clipped top-left 2x2 is blue; non-finite fills are no-ops.
    assert_eq!(&bitmap.rgba[0..4], &[0, 0, 255, 255]);
    assert_eq!(&bitmap.rgba[8..12], &[0, 0, 0, 0]);
}

#[test]
fn paint_canvases_helper_sequence_matches_wpt() {
    // Mirrors css-images `paintCanvases()`: width/height setters, getContext,
    // four fillStyle/fillRect quadrants.
    let (mut host, _, _, body) = StubHost::page();
    let canvas = host.document.create_detached_element("canvas").unwrap();
    host.document.append_child(body, canvas).unwrap();
    host.document.mark_in_document_flags();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "for (let canvas of document.getElementsByTagName('canvas')) { \
           canvas.width = 50; canvas.height = 100; \
           let ctx = canvas.getContext('2d'); \
           ctx.fillStyle = 'blue'; ctx.fillRect(0, 0, 25, 50); \
           ctx.fillStyle = 'green'; ctx.fillRect(25, 0, 25, 50); \
           ctx.fillStyle = 'red'; ctx.fillRect(0, 50, 25, 50); \
           ctx.fillStyle = 'yellow'; ctx.fillRect(25, 50, 50, 50); \
         }",
    )
    .unwrap();
    let host = rt.into_host();
    let bitmap = host
        .document()
        .canvas_bitmap(canvas)
        .expect("bitmap exists");
    assert_eq!((bitmap.width, bitmap.height), (50, 100));
    let px = |x: u32, y: u32| {
        let off = ((y * 50 + x) as usize) * 4;
        [
            bitmap.rgba[off],
            bitmap.rgba[off + 1],
            bitmap.rgba[off + 2],
            bitmap.rgba[off + 3],
        ]
    };
    assert_eq!(px(0, 0), [0, 0, 255, 255], "blue quadrant");
    assert_eq!(px(25, 0), [0, 128, 0, 255], "green quadrant (CSS green)");
    assert_eq!(px(0, 50), [255, 0, 0, 255], "red quadrant");
    assert_eq!(px(25, 50), [255, 255, 0, 255], "yellow quadrant");
}

#[test]
fn fill_style_accepts_every_supported_color_shape() {
    let mut rt = rt();
    for (input, expected) in [
        ("transparent", "transparent"),
        ("#f00", "#f00"),
        ("#f008", "#f008"),
        ("#ff0000", "#ff0000"),
        ("#ff000080", "#ff000080"),
        ("rgb(255, 0, 0)", "rgb(255, 0, 0)"),
        ("rgba(255, 0, 0, 0.5)", "rgba(255, 0, 0, 0.5)"),
        ("rgb(100%, 0%, 0%)", "rgb(100%, 0%, 0%)"),
        ("rgb(255 0 0 / 50%)", "rgb(255 0 0 / 50%)"),
    ] {
        rt.evaluate(&format!(
            "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); ctx.fillStyle = '{input}';"
        ))
        .unwrap();
        ok(&mut rt, &format!("ctx.fillStyle === '{expected}'"));
    }
    // Invalid shapes retain the previous value.
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         ctx.fillStyle = 'blue'; ctx.fillStyle = '#12'; ctx.fillStyle === 'blue' && \
         (ctx.fillStyle = 'rgb(bogus)', ctx.fillStyle === 'blue')",
    );
}

#[test]
fn context_brand_and_arity_checks_fail_closed() {
    let mut rt = rt();
    let err = rt.evaluate("CanvasRenderingContext2D.prototype.fillRect.call({}, 0, 0, 1, 1)");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "fillRect on non-context throws, got {err:?}"
    );
    for src in [
        "document.createElement('canvas').getContext('2d').fillRect(0, 0, 1)",
        "document.createElement('canvas').getContext('2d').clearRect(0, 0, 1)",
    ] {
        let err = rt.evaluate(&format!(
            "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); {src}"
        ));
        assert!(
            matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("requires 4")),
            "arity throws, got {err:?} for {src}"
        );
    }
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         ctx.canvas === c && c.getContext(null) === null && c.getContext(undefined) === null",
    );
    ok(
        &mut rt,
        "var c = document.createElement('canvas'); var ctx = c.getContext('2d'); \
         ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 0, 2) === undefined && \
         ctx.clearRect(NaN, 0, 2, 2) === undefined",
    );
}

#[test]
fn translucent_fill_blends_in_js_bitmap() {
    let (mut host, _, _, body) = StubHost::page();
    let canvas = host.document.create_detached_element("canvas").unwrap();
    host.document.append_child(body, canvas).unwrap();
    host.document.mark_in_document_flags();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var c = document.getElementsByTagName('canvas')[0]; \
         c.width = 1; c.height = 1; \
         var ctx = c.getContext('2d'); \
         ctx.fillStyle = 'blue'; ctx.fillRect(0, 0, 1, 1); \
         ctx.fillStyle = 'rgba(255, 0, 0, 0.5)'; ctx.fillRect(0, 0, 1, 1);",
    )
    .unwrap();
    let host = rt.into_host();
    let bitmap = host.document().canvas_bitmap(canvas).expect("bitmap");
    assert_eq!(bitmap.rgba.as_slice(), &[128, 0, 127, 255]);
}
