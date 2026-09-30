use super::*;

fn canvas_in_body() -> (Document, usize) {
    let mut doc = Document::new();
    let root = doc.root_index();
    let html = doc.create_detached_element("html").unwrap();
    doc.attach_child(root, html);
    let body = doc.create_detached_element("body").unwrap();
    doc.append_child(html, body).unwrap();
    let canvas = doc.create_detached_element("canvas").unwrap();
    doc.append_child(body, canvas).unwrap();
    doc.mark_in_document_flags();
    (doc, canvas)
}

#[test]
fn canvas_size_defaults_and_parses_attributes() {
    let (mut doc, canvas) = canvas_in_body();
    assert_eq!(doc.canvas_size(canvas), Some((300, 150)));
    doc.set_element_attribute(canvas, "width", "50").unwrap();
    doc.set_element_attribute(canvas, "height", "100").unwrap();
    assert_eq!(doc.canvas_size(canvas), Some((50, 100)));
    doc.set_element_attribute(canvas, "width", "bogus").unwrap();
    assert_eq!(doc.canvas_size(canvas), Some((300, 100)));
    doc.set_element_attribute(canvas, "width", "  +42  ")
        .unwrap();
    assert_eq!(doc.canvas_size(canvas), Some((42, 100)));
    doc.set_element_attribute(canvas, "width", "0").unwrap();
    assert_eq!(doc.canvas_size(canvas), Some((0, 100)));
    let div = doc.create_detached_element("div").unwrap();
    assert_eq!(doc.canvas_size(div), None);
}

#[test]
fn canvas_width_set_always_clears_even_when_unchanged() {
    let (mut doc, canvas) = canvas_in_body();
    doc.set_element_attribute(canvas, "width", "4").unwrap();
    doc.set_element_attribute(canvas, "height", "4").unwrap();
    doc.canvas_fill_rect(canvas, 0, 0, 4, 4, [255, 0, 0, 255]);
    assert!(
        doc.canvas_bitmap_ref(canvas)
            .is_some_and(|b| b.rgba[0] == 255)
    );
    doc.set_element_attribute(canvas, "width", "4").unwrap();
    let bitmap = doc.canvas_bitmap(canvas).expect("bitmap survives resize");
    assert!(bitmap.rgba.iter().all(|&b| b == 0));
}

#[test]
fn canvas_remove_missing_attribute_is_noop() {
    let (mut doc, canvas) = canvas_in_body();
    doc.set_element_attribute(canvas, "width", "4").unwrap();
    doc.set_element_attribute(canvas, "height", "4").unwrap();
    doc.canvas_fill_rect(canvas, 0, 0, 4, 4, [255, 0, 0, 255]);
    // Removing a missing attribute leaves the bitmap untouched.
    let before = doc.canvas_bitmap(canvas).unwrap();
    doc.remove_element_attribute(canvas, "data-missing")
        .unwrap();
    assert_eq!(doc.canvas_bitmap(canvas).unwrap(), before);
    // Removing an existing width reverts to 300 and clears.
    doc.remove_element_attribute(canvas, "width").unwrap();
    assert_eq!(doc.canvas_size(canvas), Some((300, 4)));
    assert!(
        doc.canvas_bitmap(canvas)
            .unwrap()
            .rgba
            .iter()
            .all(|&b| b == 0)
    );
}

#[test]
fn canvas_fill_and_clear_rect_clip_and_blend() {
    let (mut doc, canvas) = canvas_in_body();
    doc.set_element_attribute(canvas, "width", "4").unwrap();
    doc.set_element_attribute(canvas, "height", "4").unwrap();
    assert!(doc.canvas_fill_rect(canvas, -1, -1, 3, 3, [0, 0, 255, 255]));
    let bitmap = doc.canvas_bitmap(canvas).unwrap();
    assert_eq!(&bitmap.rgba[0..4], &[0, 0, 255, 255]);
    assert_eq!(&bitmap.rgba[8..12], &[0, 0, 0, 0]);
    assert!(doc.canvas_clear_rect(canvas, 0, 0, 2, 2));
    let bitmap = doc.canvas_bitmap(canvas).unwrap();
    assert_eq!(&bitmap.rgba[0..4], &[0, 0, 0, 0]);
    // Non-canvas nodes return false.
    let div = doc.create_detached_element("div").unwrap();
    assert!(!doc.canvas_fill_rect(div, 0, 0, 1, 1, [255, 0, 0, 255]));
    assert!(!doc.canvas_clear_rect(div, 0, 0, 1, 1));
}

#[test]
fn canvas_sidecar_round_trips_in_tree_order() {
    let (mut doc, first) = canvas_in_body();
    let body = doc.parent_of(first).unwrap();
    let second = doc.create_detached_element("canvas").unwrap();
    doc.append_child(body, second).unwrap();
    doc.set_element_attribute(first, "width", "2").unwrap();
    doc.set_element_attribute(first, "height", "2").unwrap();
    doc.set_element_attribute(second, "width", "3").unwrap();
    doc.set_element_attribute(second, "height", "1").unwrap();
    doc.canvas_fill_rect(first, 0, 0, 2, 2, [255, 0, 0, 255]);
    doc.canvas_fill_rect(second, 0, 0, 3, 1, [0, 255, 0, 255]);
    let sidecar = doc.canvases_in_tree_order();
    assert_eq!(sidecar.len(), 2);
    assert_eq!((sidecar[0].width, sidecar[0].height), (2, 2));
    assert_eq!((sidecar[1].width, sidecar[1].height), (3, 1));
    // A fresh document with the same shape restores bitmaps by order.
    let (mut fresh, fresh_first) = canvas_in_body();
    let fresh_body = fresh.parent_of(fresh_first).unwrap();
    let fresh_second = fresh.create_detached_element("canvas").unwrap();
    fresh.append_child(fresh_body, fresh_second).unwrap();
    fresh
        .set_element_attribute(fresh_first, "width", "2")
        .unwrap();
    fresh
        .set_element_attribute(fresh_first, "height", "2")
        .unwrap();
    fresh
        .set_element_attribute(fresh_second, "width", "3")
        .unwrap();
    fresh
        .set_element_attribute(fresh_second, "height", "1")
        .unwrap();
    fresh.set_canvases_in_tree_order(&sidecar);
    assert_eq!(fresh.canvas_bitmap(fresh_first).unwrap(), sidecar[0]);
    assert_eq!(fresh.canvas_bitmap(fresh_second).unwrap(), sidecar[1]);
}
